# figgy

Rust scientific chart library. **CPU raster (axes / labels / grid — tiny-skia + swash) + GPU wgpu (large data) hybrid** rendering.
Embed in egui / winit / any other wgpu 30 host.

> [한국어 문서](#한국어-문서) is available below.

> This is the workspace root README. The workspace has three crates:
> **`crates/model`** — the pure chart model and schema authority: option/data SSoT (`Config`, `SeriesConfig`), the rich-text/legend document model, interaction policies (`Selectable`/`Draggable`/`Resizable`, `HitMap`, the single `Config::nudge` movement path), presets (`AxisPreset`, `ColorCycle`). Dependency-free; optional `serde` feature.
> **`crates/renderer`** — the wgpu + CPU-raster machinery documented below. It owns the persistent chart registry (`ChartId` → `Config`, ordered `SeriesConfig`, selection, checked revisions), resident `ColumnPool`, nonresident logical-source metadata in the 0.12.0 candidate, picker pipeline bundle and derived single active-chart registry cache, and pending GPU-pool maintenance. Depends on `model` and re-exports its public modules.
> **`crates/web`** — the browser package (`figgy`): public `<figgy-chart>` Custom Element facade plus a raw `FiggyChart` wasm kernel as an advanced escape hatch. The facade owns the shadow canvas, ready promise/event lifecycle, rAF loop, ResizeObserver/DPR handling, pointer mapping, async-operation busy gate, id-keyed registration metadata, UI-derived labels/styles/extents, and Promise adaptation. Picker, pool, chart, and maintenance authority remain in `Renderer`. Browser I/O: [WASM.md](crates/renderer/WASM.md) · full Config JSON schema: [SCHEMA.md](crates/web/SCHEMA.md). Build artifacts (`crates/web/pkg/`) are gitignored — build with `npx wasm-pack build crates/web --release --target web`.
> **Online studio** — [figgyplot.com](https://figgyplot.com/) hosts the public web editor. It runs in-browser with local chart data, imports CSV/TSV/Excel, opens `.figgy` project files, and exports PNGs from the same wasm/WebGPU surface.

## Public release candidate — renderer 0.12.0 / figgy 0.10.0

This candidate adds renderer-owned exact nonresident streaming and the browser
`render_chart()` job API. The web facade requests bounded original ranges,
schedules work, and reports progress. For supported precise point, solid-line,
and errorbar charts, the renderer can retain only the original rows needed by
the current view in a chart-local GPU cache; it never promotes the connected
whole-column closure automatically. Wider views and scaled export replay the
source, while a narrower view can redraw from that cache.
There is no LOD or downsampling. Supported styles and exclusions are listed in
[WASM.md](crates/renderer/WASM.md). This source candidate does **not** mean the
online Studio has adopted the new API. The published release remains 0.11.0 /
0.9.1 until this candidate passes the public gate and is published.

The previous renderer 0.11.0 / figgy 0.9.1 release introduced:

Subpixel histogram bins now fill each pixel column from zero to its maximum
bin value on the GPU. An enabled, nontransparent stroke supplies the entire
fill colour; otherwise the bin's fill colour is used. Original columns and
web API signatures are unchanged. The Rust `ColumnBarLayer` has a new
`envelope` field, so downstream struct literals require an update (`None`
for manually built layers without an envelope).
The browser demo includes bin-count and stroke controls. Run
`npx serve crates/web -l 8142`, then open `http://localhost:8142/`.

Renderer 0.10.0 / figgy 0.9.0 introduced:

- **Histogram and matrix fields are first-class GPU series.** `Histogram`, `Heatmap`, `Contour`, and `ContourFill` share the declared matrix lattice and colour-map SSoT. Heatmaps support flat or interpolated shading, contours support up to 1024 levels, and contour labels open a real gap in the underlying isoline. Automatic field fitting uses the rendered cell boundaries rather than only the sample centres.
- **Field interaction and styling use stable identities.** `pick_data` returns tagged point, histogram-bin, matrix-cell, or contour-level references that can be written back through `Config.picked_data`. Histograms expose width, outline colour/thickness, and per-bin overrides. Contour label text/background/number formatting is independent of per-level line colour. The colourbar exposes its full `AxisOptions`, including ticks, labels, title, reversal, and pointer-following resize handles.
- **GPU range and startup contracts are exact and observable.** Hi/lo field-coordinate arithmetic and range reduction stay on the GPU; the reduced bounds committed to the axis SSoT are the same values used for drawing. Browser startup validates every render entry, including the contour-label width attribute, and `prewarm_all_with_progress` / `prewarm_all` can publish the renderer-owned lazy caches explicitly.
- **GPU memory is budgeted as one renderer resource.** Pool storage, staging, export, picking, contour placement, and other out-of-pool allocations are charged against the configured device-aware limit and fail with a renderer error before an unchecked allocation.

This repository is the supported source distribution; the crates are not published on crates.io. Consumers pinned to a public Git revision must update their lockfile and rebuild the wasm package. See [WASM.md](crates/renderer/WASM.md) for browser lifecycle/API details and [SCHEMA.md](crates/web/SCHEMA.md) for the complete JSON contract.

- **Resident GPU columnar pool**: columns admitted to the resident path share a single GPU buffer with first-fit alloc + ping-pong defrag on fragmentation. Logical values are stored as f32 hi/lo pairs when uploaded through `HiLoColumnSource`, preserving timestamp-sized offsets on the GPU. Upload caches scalar stats (min / max / smallest-positive) for auto-fit; per-point geometry such as the dashed-line arc-length prefix is computed in place by a compute scan (`line_arc.wgsl`).
- **Nonresident rendering (0.10.0 candidate)**: replayable columns are registered by logical ID, length, encoding, and revision without keeping their complete data in the GPU pool. The renderer requests bounded original ranges and accumulates the exact drawing; the host retains the source for replay after a view or output change. This does not decimate or downsample data. Admission, supported styles, cancellation, and completed-revision queries are described in [WASM.md](crates/renderer/WASM.md).
- **Layered compositing**: grid → data → axis/label/legend, so grid never covers the data. Axis raster can be produced as `Grid` and `Decoration` layers; `AxisLayerKind::All` remains a legacy single-pass helper.
- **Data fidelity contract**: renderer/web consume the model contract without silently changing original coordinates, provenance, or axis↔data correspondence. Explicit clipping, log-domain skips, NaN skips, and antialiasing limits are rendering contracts rather than data rewrites.
- **Headless PNG export**: GPU offscreen raster at arbitrary DPI → RGBA / PNG bytes in memory (async-first; blocking wrappers on native).
- **Interaction layer (opt-in)**: hit-testing, selection boxes, drag (axes constrained to their perpendicular, detached-axis `line_offset`), PPT-style 8-handle resize of the data area — all policy in `model`, fed by host pointer events; never runs if you don't wire it.
- **Data picking (opt-in)**: `pick_point` retains the point/line compatibility contract, while `pick_data` additionally returns tagged histogram-bin, canonical matrix-cell, and contour-level identities. Resident picking evaluates bar rectangles and field/contour geometry on the GPU from the same transform, pool, style, lattice, and level tables used to draw them. A completed chart-local packed view supports GPU point/line picking with original row indices and no source replay; other nonresident streams return `null`. Low-level WASM stream-pick replay entry points reject immediately. Hosts may still feed known stable refs through `Config.picked_data` or `Config.picked_points` for bounded selection highlighting.
- **Per-point style mapping (opt-in)**: precise scatter can bind `point_style_table` / `point_style_index_column` / `point_style_overrides`; precise errorbars can independently bind `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`. Styled modes keep their own visual shaders and ignore these mappings.
- **Rich-text everywhere**: titles, tick labels, and the legend share one engine — per-segment bold/italic/underline/sub/superscript/greek, per-segment color & size overrides, `'\n'` line breaks, `'\t'` table columns, fixed-width legend symbol fields.
- **Hand-drawn sketch mode (opt-in)**: `draw_style: { mode: "sketch", amplitude_px, wavelength_px, seed }` renders the whole chart xkcd-style — axes/ticks/grid/legend wobble on the CPU raster, line wobble/dash phase uses arc-length-scan-driven GPU variants, markers/errorbars use dedicated GPU variants, and chart text automatically switches to the bundled handwritten face (Comic Neue, OFL) with per-character fallback for glyphs it lacks (CJK keeps your registered font). Deterministic (seeded), composes with dashes, and the field's absence means the precise path runs completely untouched.
- **Milkyway mode (opt-in)**: `draw_style: { mode: "milkyway", ... }` renders the chart as an astrophotograph — lines become star chains over a series-colored nebula ribbon; scatter markers become ringed planets; errorbars become bipolar jets over a deep-space backdrop.
- **Constellation mode (opt-in)**: `draw_style: { mode: "constellation", ... }` supports `ScatterLine` series only: PSF-rendered stars sit at scatter data positions and a translucent line connects them. Parameter ranges ship as machine-readable metadata (`draw_style_param_specs`).
- **Single wgpu major (30)**: the renderer and active egui integration use wgpu 30. The retained iced integration source is not a build target because iced 0.14 still exposes wgpu 27 types.
- **WebAssembly-ready**: pure-Rust raster stack (tiny-skia + fontdb + swash), async init/export, runtime font registration (`register_font`) for CJK and custom families.
- **Observable web startup**: in a wasm browser, `create` / `create_with_progress` warm every render WGSL entry on the same `GPUDevice` through Promise-based `createRenderPipelineAsync`, discard those temporary JS pipelines, and then await the first empty-chart frame. Renderer-owned optional render/style and arc/fit/picker/contour compute caches remain lazy until first use or explicit `prewarm_all_with_progress` / `prewarm_all`. `<figgy-chart>` publishes its first successful frame and `figgy-ready` before renderer-owned GPU picking is prewarmed in the background.

### Draw style preview

Same growth-response data, rendered through the four chart styles:

<table>
  <tr>
    <td width="50%"><strong>Precise</strong><br><img src="crates/renderer/assets/style-growth-response-precise.png" alt="Precise style growth-response chart" width="420"></td>
    <td width="50%"><strong>Sketch</strong><br><img src="crates/renderer/assets/style-growth-response-sketch.png" alt="Sketch style growth-response chart" width="420"></td>
  </tr>
  <tr>
    <td width="50%"><strong>Milkyway</strong><br><img src="crates/renderer/assets/style-growth-response-milkyway.png" alt="Milkyway style growth-response chart" width="420"></td>
    <td width="50%"><strong>Constellation</strong><br><img src="crates/renderer/assets/style-growth-response-constellation.png" alt="Constellation style growth-response chart" width="420"></td>
  </tr>
</table>

---

## 1. Usage

### Adding the dependency

```toml
[dependencies]
renderer = { path = "crates/renderer" }   # or public Git source — candidate 0.12.0, not on crates.io.
wgpu     = "30"
```

The library itself depends on neither winit, egui, nor iced. Pull in only the host you actually use:

```toml
# winit standalone
winit = "0.30"

# egui embedded
eframe    = { version = "0.36", default-features = false, features = ["wgpu"] }
egui      = "0.36"
egui-wgpu = "0.36"
```

iced 0.14 still uses wgpu 27, so direct device/queue/render-pass sharing is
disabled until iced publishes a wgpu 30-compatible release.

### Shortest standalone example (winit + figgy alone with wgpu)

```rust
use std::sync::Arc;
use renderer::{
    Chart, ChartDrawItem, DataLineStyleConfig, DataRenderType, Renderer, Series, SeriesConfig,
    color::Color, default, layout::{ChartArea, Rect}, line::LineStylePreset,
};

let window = Arc::new(event_loop.create_window(attrs).unwrap());
let size = window.inner_size();

// One-line setup — figgy owns instance/adapter/device/queue/surface/swap chain.
let mut renderer = Renderer::for_window(
    Arc::clone(&window),
    (size.width, size.height),
    16 * 1024 * 1024,   // 16 MiB GPU column pool
).unwrap();

// renderer.add_column takes `&dyn ColumnSource`.
// Implement the trait on your own type (see `ColumnSource` section below) — Vec, ndarray,
// polars Series, mmap, anything — and you get zero-copy upload. Built-in `Column<f64>` works too.
let xs: Vec<f64> = (0..1024).map(|i| i as f64 * 0.01).collect();
let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
renderer.add_column("x", &my_source_for(0, xs)).unwrap();   // your type : ColumnSource
renderer.add_column("y", &my_source_for(1, ys)).unwrap();

// Chart — builder pattern.
let mut config = default::default_config();
config.chart_area = ChartArea(Rect { x:8, y:8, width: size.width - 16, height: size.height - 16 });
let mut chart = Chart::new(config)
    .with_title("Sine")
    .with_x_title("x")
    .with_y_title("sin(x)");
chart.auto_fit_x(renderer.pool(), "x", 0.05).unwrap();
chart.auto_fit_y(renderer.pool(), "y", 0.10).unwrap();

// Series = SeriesConfig (declaration) + ChartStyle (GPU style auto-built from that declaration).
let cfg = SeriesConfig {
    series_id: "sin".into(), label: None,
    source_id: None,
    x_column: "x".into(), y_column: "y".into(),
    render_type: DataRenderType::Line {
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::from_rgb8(20, 110, 230),
            line_width: 2.0,
        },
    },
};
let style = renderer.create_style_for_series(&cfg);            // SeriesConfig → ChartStyle
let view  = renderer.create_chart_view(&chart, chart.config().chart_area.0).unwrap();

// frame loop:
let series = [Series { config: &cfg, style: &style }];
let items  = [ChartDrawItem {
    view: &view,
    chart_config: chart.config(),
    series: &series,
}];
renderer.draw(Color::WHITE, &items).unwrap();   // acquire surface frame → prepare → encoder → pass → paint_prepared → submit → present
```

### `ColumnSource` — the data adapter trait

`Renderer::add_column` takes `&dyn ColumnSource` — implement the trait on any container of yours and the data lands in the GPU pool with zero copy (no intermediate `Vec` allocation). The source writes GPU pairs and returns smallest-positive statistics in the same pass; the renderer never reads wgpu 30's write-only mapped bytes. `min` / `max` retain their source-level meaning. Scalar smallest-positive uses the actual uploaded `(value as f32, 0)` value, while hi-lo uses the recorded `hi as f64 + lo as f64`; both include only finite positive values. Use `Renderer::add_hilo_column` with `&dyn HiLoColumnSource` for large absolute timestamps or coordinates that must preserve sub-f32 deltas.

`add_column` / `add_hilo_column` register a new id. To atomically replace an
existing id, use `upsert_column` / `upsert_hilo_column`. `Renderer` prepares the
provisional pool, affected chart revisions, active picker transition, and
maintenance state before publishing any of them. A returned preparation error
therefore preserves the previous authority state. Integrations with additional
host-owned derived state may use `begin_upsert_*`, inspect its provisional pool,
prepare that derived state, and then call the guard's infallible,
allocation-free `commit`; hosts do not rebuild the picker themselves.

```rust
pub trait ColumnSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }  // default
    fn min(&self) -> f64;
    fn max(&self) -> f64;

    /// Legacy scalar encoder retained for source compatibility.
    /// Caller guarantees `dst.len() == self.len() * 4`. null → `f32::NAN`.
    fn write_f32_le_into(&self, dst: &mut [u8]);

    /// Write `(value as f32, 0)` pairs and return stats in the same pass.
    fn write_f32_pair_le_into_with_stats(
        &self,
        writer: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats;
}
```

**Built-in implementors**: `Column<f64>`, `Column<f32>`, `Column<Option<f64>>` (null → NaN).

```rust
pub trait HiLoColumnSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }
    fn min(&self) -> f64;
    fn max(&self) -> f64;

    /// Legacy slice encoder retained for source compatibility.
    /// Caller guarantees `dst.len() == self.len() * 8`.
    fn write_f32_pair_le_into(&self, dst: &mut [u8]);

    /// Write pairs and return stats from recorded `hi as f64 + lo as f64`.
    fn write_f32_pair_le_into_with_stats(
        &self,
        writer: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats;
}
```

`Column<f64>` implements `HiLoColumnSource`; browser hosts should pass a
`Float64Array` through `register_column_f64` for the first upload and
`update_register_column_f64` for an explicit replacement when using timestamp
axes with large Unix epoch values.

Custom trait implementations must implement the fused method. This makes an
incomplete migration a compile-time error instead of allowing a source to
compile and then fail during upload. There is no byte-readback or silently
incorrect fallback.

**Custom — time series / DataFrame / mmap / FFI data, anything**:

```rust
struct MyTimeSeries {
    samples: Vec<f64>,    // or Arc<[f64]>, ndarray::ArrayView, polars::Series, ...
    cached_min: f64,
    cached_max: f64,
}

impl renderer::ColumnSource for MyTimeSeries {
    fn len(&self) -> usize { self.samples.len() }
    fn min(&self) -> f64 { self.cached_min }
    fn max(&self) -> f64 { self.cached_max }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.samples.len() * 4);
        for (i, &v) in self.samples.iter().enumerate() {
            dst[i*4..i*4+4].copy_from_slice(&(v as f32).to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut writer: renderer::ColumnPairWriter<'_>,
    ) -> renderer::ColumnUploadStats {
        debug_assert_eq!(writer.len(), self.samples.len());
        let mut min_positive: Option<f64> = None;
        for (index, &sample) in self.samples.iter().enumerate() {
            let value = sample as f32;
            writer.write_pair(index, value, 0.0);
            let value = value as f64;
            if value.is_finite() && value > 0.0
                && min_positive.map_or(true, |current| value < current)
            {
                min_positive = Some(value);
            }
        }
        renderer::ColumnUploadStats { min_positive }
    }
}

renderer.add_column("temperature", &my_series)?;   // ↘ writes directly into mapped staging memory, zero Vec
```

Native `f32` containers use the same fused path: iterate the values and call
`writer.write_pair(index, value, 0.0)`. `ColumnPairWriter` exposes logical pair
writes rather than mapped bytes, so active pool upload remains allocation-free
without exposing wgpu or permitting a `dst.copy_from_slice(...)` shortcut.

### Native examples — sine / RC / cross-section

```bash
cargo run -p renderer --example winit_simple
cargo run -p renderer --example egui_embed --features egui_demo
```

Each example shows:
- A 3-panel grid with different grid options (off / major / major + dotted minor)
- The RC panel renders 2 series (charging + discharging)
- Line widths of 1 / 2 / 3.5 px across panels
- Legends
- DPI input + Save PNG button (egui) or `S` key (winit) → per-panel PNG bytes in memory → written by the example to `/tmp/figgy_*_panel_{i}.png`

### Browser timestamp-axis demo

`crates/web/timestamp-demo.html` exercises the browser timestamp path with
absolute Unix time values uploaded through `register_column_f64(Float64Array)`
and explicitly replaced through `update_register_column_f64`.
It lets you change the visible time window, data unit, timezone, fractional
second policy, label pattern, chart width, and export scale while the x axis
uses `LabelFormat::Timestamp` + `AutoCalendar` to choose non-overlapping labels.

```bash
npx wasm-pack build crates/web --release --target web
cd crates/web
python -m http.server 8142 --bind 127.0.0.1
# open http://127.0.0.1:8142/timestamp-demo.html
```

### Live SSoT lab — the split API at pool scale

```bash
cargo run --release -p renderer --example ssot_lab --features egui_demo
```

A 2×2 grid, one draw style per panel (Precise dashed / Sketch / Milkyway /
Constellation), all four series reading a single shared `x` pool column. The
sidebar edits the SSoT live — pan direction per x-linked column pair, window
width, and point density up to 3M/series (12M total, 5 columns). Every edit
flows through `Renderer::prepare` → `Renderer::paint_prepared` with no
`Mutex` and no per-frame `update_transform`; the status box's `frames
skipped` stays at 0, proving the token never goes stale under live edits.

### egui integration pattern (summary)

The frame is split to match host callback shapes: every mutation lives in
`Renderer::prepare` (`&mut self`), pure command recording lives in
`Renderer::paint_prepared` (`&self`) — so the paint callback needs no
`Mutex` around the renderer:

```rust
// stored in CallbackResources as plain FiggyState — no Mutex
struct FiggyState { renderer: renderer::Renderer, panels: Vec<PanelEntry> }
struct PanelEntry { /* chart, view, … */ prepared: Option<renderer::PreparedFrame> }
struct FiggyCallback { panel_idx: usize /* one callback per panel */ }

impl egui_wgpu::CallbackTrait for FiggyCallback {
    fn prepare(&self, _device, _queue, _screen, _enc, resources) -> Vec<...> {
        let state = resources.get_mut::<FiggyState>().unwrap();
        // dirty handling: refresh_axis (raster). No per-frame update_transform —
        // prepare writes the transform uniform itself.
        let prepared = state.renderer.prepare(&items).unwrap();
        // store the token per panel — egui runs every callback's prepare before
        // any paint, so one shared token would be overwritten by later panels
        state.panels[self.panel_idx].prepared = Some(prepared);
        Vec::new()
    }
    fn paint(&self, info, render_pass, resources) {
        let state = resources.get::<FiggyState>().unwrap();
        let prepared = state.panels[self.panel_idx].prepared.as_ref().unwrap();
        let target = (info.screen_size_px[0], info.screen_size_px[1]);
        state.renderer.paint_prepared(render_pass, target, prepared).unwrap();
    }
}
```

After the host submits every command buffer for the frame, call
`renderer.end_gpu_frame()` exactly once. With callback schedulers such as egui,
the frame coordinator may make that call before the first prepare of the next
frame, after the previous frame is known to have been submitted. It must not be
called once per panel prepare: other panels may still hold recorded but
unsubmitted resources.

Renderer-owned submit paths (`WindowedRenderer::draw*` and panel export) report
that boundary themselves. A host that also records external passes must submit
those pending command buffers before invoking one of these paths; wgpu command
buffers are opaque, so the renderer cannot identify or retire only one host's
pending references.

`paint_prepared` is repeatable (the same token may be recorded into more than
one pass). `PreparedFrame` owns the resolved draw inputs, so paint neither
reconstructs nor receives `items`. If a captured renderer resource changes
between the two phases, paint records nothing and returns
`FiggyError::StalePreparedFrame` — recover with a fresh `prepare` next frame.
Automatic contour labels follow the same ownership rule per panel/item + series
occurrence: the baked atlas and cell table are immutable cache resources, while
each distinct dispatch input produces one immutable placement result owning its
params, transform, candidates, anchors, indirect args, compute bind groups, and
GPU charge. An exact key reuses that result; a different key never rewrites it,
even after the token drops, because a host command buffer may still hold the old
GPU handles before submission. Explicit anchors are one immutable placement
snapshot. Arc/star compute results follow the same exact-key rule. Paint uses
the token's exact resources and never re-reads the series cache.

The host must submit a command buffer recorded from a token before the next
mutation of that token's `ChartView` (`refresh_axis`, `update_transform`, or a
later `prepare` using the same view). The content revision rejects stale input
before recording; it cannot inspect or order a command buffer after ownership
has moved to the host.
The one-shot `Renderer::paint(&mut self, …)` facade remains for hosts that own
the renderer exclusively during their frame (winit loop, wasm wrapper): it
runs both phases back to back.

Full version: [examples/egui_embed.rs](crates/renderer/examples/egui_embed.rs).

### iced integration status

The retained [iced integration source](crates/renderer/unsupported/iced_embed_wgpu27.rs)
documents the intended `prepare` / `paint_prepared` ownership pattern, but its
build target is disabled while iced 0.14 remains on wgpu 27. wgpu device,
queue, and render-pass types cannot be shared across major versions.

### PNG export (memory only — saving is the caller's job)

```rust
let bytes = renderer
    .export_panel_png_bytes_async(&chart, &series_configs, scale)
    .await?;
std::fs::write("/tmp/out.png", &bytes)?;          // or clipboard / network / wherever.

// If you only need RGBA:
let img = renderer
    .export_panel_rgba_async(&chart, &series_configs, scale)
    .await?;
// img.width, img.height, img.rgba (straight alpha, length = w * h * 4)
```

Native-only blocking convenience wrappers use the same names without `_async`.
`scale` bounds: `renderer::MIN_EXPORT_SCALE` (0.25) ~ `renderer::MAX_EXPORT_SCALE` (8.0), automatically clamped.
Convert from standard 96 DPI via `renderer::dpi_to_scale(dpi)`.

When scaling, every pixel-based dimension (font / line / margin / grid / legend) scales proportionally → the visual is identical, just denser pixels.

---

## 2. Config struct field reference

```rust
pub struct Config {
    pub chart_area: ChartArea,           // panel pixel rect (inside the host viewport)
    pub top_x: AxisOptions,              // 4-side axes — top/right labels & titles disabled by default
    pub bottom_x: AxisOptions,
    pub left_y: AxisOptions,
    pub right_y: AxisOptions,
    pub chart_title: ChartTitleOptions,
    pub grid: GridOptions,
    pub legend: Legend,
    pub picked_points: Option<PickedPointsConfig>,
    pub picked_data: Option<DataSelectionsConfig>,
    pub colorbar: Option<ColorBarOptions>,   // the colourbar AND the chart's z scale
    pub draw_style: DrawStyle,
}
```

### `ChartArea` / `Rect`
| Field | Type | Meaning |
|---|---|---|
| `x, y` | u32 | Top-left pixel position relative to the host surface |
| `width, height` | u32 | Panel pixel size. 0 → live raster fails (`InvalidChartArea`); callers should keep export chart areas non-zero too. Export's current 1 px clamp is a compatibility guard and may become an explicit error |

### `AxisOptions` (top_x / bottom_x / left_y / right_y)
| Field | Type | Meaning |
|---|---|---|
| `scale` | `AxisScale` | `Linear` or `Logarithmic` (log10) |
| `min, max` | f64 | Data-space range. For log scale, positive bounds are used as-is; manual non-positive/non-finite bounds are guarded to `1e-12` on renderer/axis paths. Non-positive data samples are skipped/NaN-handled rather than making the whole range invalid |
| `major_spacing` | f64 | linear: data units; log: decade step (1, 2, …) |
| `minor_count` | usize | minors per major (linear) or sub-decade 2..9 (8 recommended for log) |
| `inverted` | bool | Reverses the visual direction of this axis. Tick/grid placement, data rendering, and picking all use the same reversed mapping; `min`/`max` remain the data-space bounds |
| `label_style` | `LabelStyle` | Tick-label styling |
| `tick` | `TickVisibility` | `None / Outside / Inside / Both` |
| `title_option` | `AxisTitleOptions` | Axis title text / visibility / offset |
| `out_margin` | f32 | Outer (label + title band) pixel margin |
| `line_visible / color / width / style` | mixed | Axis line appearance. CPU raster strokes floor to 1 px, so sub-pixel widths do not disappear |
| `line_offset` | f32 | Detached-axis offset: shifts the axis chrome (line/ticks/labels) perpendicular to itself while the data area stays put. Layout-neutral; the drag system's axis movement lands here |
| `major_tick_length / minor_tick_length` | f32 | Tick mark length (px) |

### `LabelStyle`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | Overall label visibility |
| `color` | `Color` | Label color |
| `font_size` | f32 | px |
| `label_visible` | bool | Number labels themselves (separate from `visible`, e.g. show the axis but hide labels) |
| `label_font` | String | Font family. Empty string → bundled Liberation Sans |
| `label_offset_x / y` | f32 | Fine nudge offset (px) |
| `format` | `LabelFormat` | `Decimal / Power / Scientific / Timestamp`. `Timestamp` interprets numeric values as Unix epoch time on linear axes and can use calendar-aware ticks |
| `significant_digits` | u8 | |

`LabelFormat::Timestamp` keeps data coordinates numeric. Its default config is
UTC Unix seconds with `AutoCalendar` tick planning; use
`unit = Milliseconds` for JS timestamps and `FixedOffsetMinutes(540)` for KST.
`AutoCalendar` measures tick-label text and coarsens the calendar step so labels
do not overlap. Use `Renderer::add_hilo_column` (native) or
`register_column_f64` / `update_register_column_f64` (web `Float64Array`) for
high-resolution absolute Unix timestamps; those paths preserve sub-f32 deltas
on the GPU as f32 hi/lo pairs.
The browser demo at [crates/web/timestamp-demo.html](crates/web/timestamp-demo.html)
is the quickest visual check for range changes, chart-width changes, and export
scale against that contract.

### `AxisTitleOptions` / `ChartTitleOptions`
| Field | Type | Meaning |
|---|---|---|
| `text` | `RichText` | greek / sub/super / bold/italic styled segments |
| `visible` | bool | |
| `offset_x / y` | f32 | nudge |
| `top_margin` | f32 | (chart_title only) chart-title band height |

### `GridOptions`
| Field | Type | Meaning |
|---|---|---|
| `show_major_x/y` | bool | Major grid lines |
| `major_x/y_color, _width, _style` | mixed | Major line appearance (Solid / Dash / Dot, 11 presets) |
| `show_minor_x/y` | bool | Minor grid lines |
| `minor_x/y_color, _width, _style` | mixed | Minor line appearance |

### `DrawStyle`
| Variant / JSON mode | Meaning |
|---|---|
| `Precise` / omitted or `{ "mode": "precise" }` | Default scientific renderer; serialized default omits `draw_style` |
| `Sketch` / `{ "mode": "sketch", ... }` | Hand-drawn chart-wide style |
| `Milkyway` / `{ "mode": "milkyway", ... }` | Astrophotograph chart-wide style. Parameter metadata comes from `draw_style_param_specs("milkyway")` |
| `Constellation` / `{ "mode": "constellation", ... }` | ScatterLine-only star chart style. Parameter metadata comes from `draw_style_param_specs("constellation")` |

### `Legend`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | |
| `content` | `RichText` | The whole legend as **one rich document**: `'\n'` segments break lines, symbols are inline segments (glyph char + per-segment `color` override) — breaks, symbol positions, and mid-text symbols are all explicit in the SSoT. `font` / `font_size` are live at draw time |
| `corner` | `LegendCorner` | `TopLeft / TopRight / BottomLeft / BottomRight` |
| `padding` | f32 | Legend box internal padding. Corner placement uses the fixed data-area inset plus `offset_x / offset_y` |
| `bg_color, border_color` | `Color` | Box background / border |

Symbols are **fixed-width field segments** (`field_em`): every form spans
exactly `SYMBOL_FIELD_EM` (2.0 em × font size) regardless of shape — a line
mark is a drawn rule (`rule: true`) filling the whole field, a scatter mark
is the shape glyph (`● ■ ▲ …`) centered in it, and line+scatter is
rule–glyph–rule summing to the same width. Dashed/dotted line styles are
carried by `rule_dash` on rule segments, so legend marks reflect
`LineStylePreset` as well as color and shape. Auto-built entries are
`symbol + ' ' + '\t' + label`, so labels also align via the tab column.
Composition helpers: `symbol_segments(kind, color)`,
`series_symbol_segments(cfg)`, `append_legend_entry(content, symbol, label)`.

### `PickedPointsConfig`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | Enables/disables the overlay when `picked_points` is present |
| `refs` | `Vec<PickedPointRef>` | Picked data references: `series_id`, optional `source_id`, and `point_index`. The overlay stores provenance, not copied coordinates |
| `ring_color` | `Color` | Highlight ring color |
| `ring_width_px` | f32 | Ring stroke width in pixels |
| `radius_extra_px` | f32 | Extra radius added around the source marker |

Missing `picked_points` / JSON `null` means no picked-point overlay. JSON `{}` is accepted as the default overlay config (`visible: true`, empty refs, gold ring, 2 px stroke, +3 px radius), so hosts can turn the overlay on and then fill `refs`.
The overlay ring follows the picked scatter marker radius, including per-point style mapping; for line-only picks it uses `radius_extra_px` around the snapped endpoint.

### `DataSelectionsConfig`

`Config.picked_data` stores tagged `PickedDataRef` identities: `Point`,
`HistogramBin`, `MatrixCell`, or `ContourLevel`. Every ref has `series_id` and
optional `source_id`; its kind then carries `point_index`, `bin_index`, canonical
`x_index/y_index`, or `level_index + x_index/y_index`. Visual policy is
`highlight_color`, `outline_width_px`, `point_radius_extra_px`, and
`contour_width_extra_px`. No ref stores coordinates, bar bounds, or contour
segments. The overlay resolves current geometry from the same GPU resources as
the normal draw, and stale/out-of-range indices draw nothing. JSON `null`
clears it and `{}` selects the default empty gold overlay.

### `ColorBarOptions`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | `false` draws nothing **and reserves no band** — the space returns to the data area. Field series still render |
| `side` | `Side` | `Left`/`Right` = vertical bar, `Top`/`Bottom` = horizontal. This alone decides the orientation |
| `thickness_px` | f32 | The strip's short dimension |
| `gap_px` | f32 | Space between the data area and the strip |
| `length_frac` | f32 | Strip length as a fraction of the data area's length along that side; must be in `(0, 1]` |
| `align` | `BarAlign` | `Start` / `Center` / `End` along that side — the discrete half of the anchor |
| `offset_x`, `offset_y` | f32 | Free offset from that anchor, in screen pixels. **Margin-noncontributing**, the same contract as `Legend::offset_{x,y}` and the title / label offsets, so nudging the bar moves it without reflowing the data area. This is where a drag accumulates |
| `colormap` | `ColorMap` | `Viridis` / `Magma` / `Turbo` / `GrayScale` / `RdBu` / `Custom { stops }` |
| `nan_color` | `Color` | Colour for z the ramp cannot place: NaN, and non-positive z on a log colourbar. Fully transparent by default |
| `border_color`, `border_width` | `Color`, f32 | Strip border |
| `axis` | `AxisOptions` | **The z axis — the single source of the z range** |

`axis` being a full `AxisOptions` is the design, not incidental reuse: `scale`
(including `Logarithmic`), `min`/`max`, `major_spacing`, `minor_count`,
`label_style` (including `LabelFormat::Power`), `tick`, and `title_option` mean
exactly what they mean on a chart axis, and tick generation, label formatting,
and log handling are the same code rather than a parallel implementation.

Consequences, all of them decisions:

- A `Heatmap` / `Contour` / `HeatmapContour` series **requires** this to be
  `Some`. Without it there is no z range and no colormap anywhere, so there is
  no value to draw — the renderer rejects the series rather than inventing one.
- There is therefore **one z scale per chart**; several heatmaps share it.
- The band is `gap_px + thickness_px + axis.out_margin + axis.major_tick_length`
  on its own side. `fit_to_data_area` / `resize_chart_area_scaled` may reclaim
  `axis.out_margin` (label space, like an axis') but never the strip itself, so
  a colourbar does not get thinner with the window.
- Two defaults differ from a chart axis: `line_visible: false` (the strip's
  border is that edge) and `tick: Outside` (ticks sit in the label margin
  instead of over the colours).
- The band is the **outermost** part of its side's margin: from the chart edge
  inward it is the bar's label margin, its ticks, the strip, `gap_px`, and only
  then that side's axis band. Axis tick labels are drawn from the data area
  outward and cannot move, so a strip placed next to the data area would be
  drawn on top of them.
- The bar is drawn on the CPU in the decoration layer, with no GPU pipeline. Its
  ticks, labels, and title go through the same helpers the four chart axes use,
  so a logarithmic colourbar gets decade ticks and 10ⁿ labels from the code that
  already does that for a logarithmic axis. `axis.tick` controls
  inside/outside/both, `axis.inverted` controls the min→max screen direction,
  and tick paint now honors the same `line_color` / `line_width` /
  `line_style` as its axis line.
- It is a **selectable, draggable, resizable** element like the rest of the
  chrome: hit-test id `"colorbar"`, a blue selection box, and — with the data
  area, the only two that have them — the eight resize handles. A drag lands in
  `offset_{x,y}`; a handle drives `thickness_px` or `length_frac` depending on
  the *bar's* orientation, which nudge resolves because a handle only knows
  screen directions. Its details are independent foreground targets:
  `"colorbar_axis"`, `"colorbar_tick_labels"`, and `"colorbar_title"`.
  Dragging them updates `axis.line_offset`, label offsets, and title offsets
  respectively. All three derive from the painted strip rect; a shortened,
  aligned, moved, or resized strip therefore keeps its title and hit geometry
  attached.
- `ColorBarOptions::normalized_z(z) -> Option<f32>` is the one z→colour
  normalization (`color_for_z` applies it): clamped to `[0, 1]`, `None` for NaN,
  for non-positive z on a logarithmic bar, and for a degenerate range — those
  draw as `nan_color` rather than clamping to an endpoint, because "missing" and
  "smallest" are different facts. `axis.inverted` is not applied: it moves where
  a value is drawn, not which colour it has.

### `data_config` — declarative series schema (the active API)

Series are declared via `data_config::SeriesConfig`. `Renderer::paint` branches on the `render_type` enum to spawn line / scatter / errorbar layers automatically; colors, widths, and shapes are also extracted from the matching sub-style.

| Type | Fields | Role |
|---|---|---|
| `SeriesConfig` | `series_id, source_id?, label, x_column: ColumnId, y_column: ColumnId, render_type` | Full series declaration. `source_id` is optional host provenance for picking; `x_column / y_column` are renderer-registered ids, resident in the pool or backed by a replayable nonresident source. In the web editing flow, `legend.content` is the live label authority; ordinary series edits update recognized legend symbols only and preserve user text. `SeriesConfig.label` becomes authoritative only for an explicit `reset_legend_from_series_labels()` rebuild |
| `DataRenderType` | enum, 13 variants | One independent draw path per variant. Optional struct merging avoided |
| `ErrorRef` | `Symmetric { column }` or `Asymmetric { lower, upper }` | Errorbar column reference. Symmetric = ±σ, Asymmetric = lower/upper split |
| `DataLineStyleConfig` | `line_style, line_color, line_width` | Line appearance |
| `DataScatterStyleConfig` | `point_color, point_shape, point_size, point_style_table?, point_style_index_column?, point_style_overrides?` | Point appearance. The optional style map applies only to precise scatter; each table/override slot can replace color, shape, size, or any subset |
| `DataErrorBarStyleConfig` | `error_bar_color, _width, _cap_size, cap_width, error_bar_style_table?, error_bar_style_index_column?, error_bar_style_overrides?` | Errorbar appearance. The optional style map applies only to precise errorbars; each table/override slot can replace color, stem width, cap half-size, cap width, or any subset |
| `DataBarStyleConfig` | `fill_color, border_color, border_width, baseline, gap_px, width_ratio, orientation, bar_style_overrides?` | Histogram appearance. `width_ratio` controls the centred fraction of each bin; sparse overrides replace fill, outline, gap, or width for selected bin indices |
| `ScatterShape` | enum, 26 variants | Circle / Square / Triangle directions / Diamond / Cross / Plus / Pentagon / Hexagon / Octagon / Star + filled variants |

**The 13 `DataRenderType` variants**:

| Variant | Sub-styles used | Meaning |
|---|---|---|
| `Line { line }` | line | Line only |
| `Scatter { scatter }` | scatter | Points only |
| `ScatterLine { scatter, line }` | both | Points + connecting line |
| `ScatterErrorbarX { scatter, err_x, err_style }` | scatter + errorbar | Points + X errorbars |
| `ScatterErrorbarY { scatter, err_y, err_style }` | scatter + errorbar | Points + Y errorbars |
| `ScatterErrorbarXY { scatter, err_x, err_y, err_style }` | scatter + errorbar | Points + X/Y errorbars |
| `LineScatterErrorbarX / Y / XY` | line + scatter + errorbar | The above + connecting line |
| `Histogram { bar }` | bar | Bars from host-binned `(edges, counts)`. `bar.orientation` alone decides which column is which: `Vertical` = `x_column` edges / `y_column` counts, `Horizontal` the reverse. The `edges = counts + 1` length relation is never used to guess |
| `Heatmap { matrix, fill }` | fill | Filled field only |
| `Contour { matrix, contour }` | contour | Contour lines only |
| `HeatmapContour { matrix, fill, contour }` | fill + contour | Filled field with lines over it |

Histogram width is resolved in two stages: `width_ratio` keeps a centred
`0..=1` fraction of the bin, then `gap_px` removes a fixed screen-space amount.
The gap is capped to leave at least one pixel for wider positive-width bars.
Bins whose original projected width is below one pixel use a GPU envelope:
each overlapping pixel column takes the maximum bin value and is filled to
zero (clipped to the visible axis range). Horizontal histograms use pixel rows.
The winning bin supplies the stroke colour for the entire area when its stroke
has positive width and alpha; otherwise it supplies the fill colour. Equal maxima use the first
bin. Gap and positive width ratios do not cut holes in this envelope;
`width_ratio = 0` still hides the bin. Original columns are unchanged.
`border_width = 0` disables the outline; otherwise `border_color` and
`border_width` control it. `bar_style_overrides` is a sparse declaration-order
list keyed by `index`; each record may independently replace `fill_color`,
`border_color`, `border_width`, `gap_px`, or `width_ratio`. Baseline and
orientation stay series-wide. Rendering, typed picking, and selected-bin
outlines all resolve the same final bar rectangle.

The three matrix variants declare their grid as `MatrixRef { columns,
orientation, grid_layout }` — a bundle of registered column ids and nothing
else. There is no separate matrix container: resident drawing reads those columns
from the pool, while supported nonresident Heatmap drawing replays their registered
sources in bounded ranges. Neither path duplicates `Config` or `series`.
`grid_layout` says whether the coordinate
columns are cell `Edges` (n + 1) or `Centers` (n); it is never inferred from the
lengths. A declaration that does not line up with the data is **not an error** —
the smallest common extent is drawn and the truncation is reported.

<!-- contour-contract: scope=readme-en max-levels=1024 -->
`ContourConfig.levels` is always an explicit list in data units (there is no
"auto" variant — a level set inferred at draw time is a value the config does not
contain). Its accepted length is `0..=1024`; 1025 or more is an error, and no
level is silently truncated. On a cache miss the renderer keeps the original
list and declaration order, while building a lookup copy from consecutive
32-level blocks sorted by value. Each fragment searches at most 32 blocks and
runs coverage math only for reachable candidates; if all 1024 levels actually
cross one cell, all 1024 are composited in declaration order. `per_level_color:
None` means every level uses `line.line_color`;
colours are not derived from the colormap. The lines themselves are drawn as the
level set of the field's own bilinear interpolation, from its analytic gradient —
so a band boundary and the line over it come out of one computation. Stroke
distance is the quadratic crossing obtained by restricting the current cell's
bilinear field to the current gradient-normal line. It is not a global shortest
distance to the whole piecewise-bilinear contour.

Non-finite levels have explicit uploaded-f32 semantics. Contour lines exclude
NaN and both infinities. For `FillMode::Bands`, the numerator counts every
`-Infinity` plus each finite level less than or equal to z, while the denominator
keeps the full declared level count:
`t=(negative_infinity_count + finite_le_z + 0.5)/(declared_level_count + 1)`.
NaN and `+Infinity` therefore affect only the denominator.

`ContourLabelConfig.anchors` is an **override**. Empty is the normal case: the GPU
places the labels itself, seeding a lattice at `spacing_px` over the data area and
projecting each seed onto its level's isoline. Normal selection targets
`spacing_px` separation, but the per-level fallback may keep a closer candidate
rather than omit a level. `spacing_px` must always be finite and greater than zero,
including for hidden labels and explicit overrides. Automatic and explicit
placement share a 1024-label capacity. Explicit anchors whose `level_index` is
invalid are discarded, and only the first 1024 valid anchors are retained in
input order. If that resolved list is empty, automatic placement runs; otherwise
the resolved list overrides it. Only automatic placement multiplies spacing by
the frame/export scale; export validates that product after clamping the scale.
The atlas must also fit the adapter's texture-dimension limit. Invalid spacing,
scale-product overflow, or an oversized atlas fails before publishing renderer
state, so the previous chart and GPU resources remain active. An anchor is data
coordinates plus a data-space tangent, so a zoom or pan only re-projects it.
`ContourLabelConfig.color` owns the text colour independently of the line and
`per_level_color`; changing a ramp never recolours the typography. Decimal text
uses the contour-level interval (falling back to the colourbar interval), never
an x-axis interval, and `significant_digits` remains effective without allowing
adjacent levels to collapse to one string. The contour fragment reads the same
selected-anchor buffer as the label draw and omits stroke coverage inside each
label rectangle. `bg_padding_px` pads that real line gap whether `bg_color` is
opaque or absent.

`Renderer::series_draw_info(chart, series_id) -> SeriesDrawInfo` is the
series-common window onto what actually drew: `drawn_count`, a matrix' `cols` /
`rows`, and `truncated`. Column lengths are facts that arrive with the data, not
SSoT, so a mismatch never errors and never stops the draw — the smallest common
extent is drawn and reported here. That is how a host learns its 11-edge /
9-count histogram drew 9 bars, and it explains the existing `min(x, y)`
truncation of lines and scatters through the same call.

These four render types do not go through the point-only compatibility picker.
Histograms fit from their uploaded edge/value metadata. Matrix fields use the
GPU fit engine in a distinct field mode: the CPU supplies only the resolved cell
counts, and the GPU reads the same coordinate-pair pool as the field shader and
applies the same `Edges`/`Centers` plus cell/sample-lattice rule. Thus contour and
interpolated-field auto-fit stops at the actual sample endpoints (edge-coordinate
midpoints for `Edges`), while a flat field fits its actual cell boundaries. The
paired reducer works on index-aligned `(x[i], y[i])`
pairs, which a histogram (`edges` is one longer) and a grid (two independent
coordinate axes) are not. `pick_data` handles them through bar/field shader
entries; fitting follows the histogram/field rules above.

**`Renderer::create_style_for_series(cfg)`** extracts color/width/shape from `cfg.render_type`'s sub-styles and builds a GPU `ChartStyle` for screen paint. For export, `create_style_for_series_scaled(cfg, scale)` scales pixel widths only.

**Single-direction errorbar** (`ScatterErrorbarY` etc.): direction presence is encoded in `PrimitiveStyle::primitive_flags` (Y=bit 0, X=bit 1). The inactive vertex slots reuse the already-bound anchor column and are collapsed before their error attributes are read, so prepare/export creates no hidden filler column and no host-maintained metadata. A real zero error remains a present, zero-length errorbar rather than being mistaken for an absent direction. (Symmetric variants reuse the same error column for lo/hi.)

### `Config::scaled(scale)` / `Config::scale_in_place(s)`
Multiplies every pixel-based dim by `scale`. `min/max/major_spacing`, scale enum, and colors are untouched. Used for resolution-invariant high-DPI export.

### Default builder — `renderer::default::default_config()`
- bottom_x / left_y: axis line + ticks + labels + title enabled, text starts as empty segments.
- top_x / right_y: axis line + ticks enabled, labels + title disabled, `out_margin = 8` (narrow gap).
- chart_title: visible, `top_margin = 32`, text empty.
- grid: major only, light gray.
- legend: disabled.

Empty text is filled in via the `Chart::with_title / with_x_title / with_y_title / with_legend_entry` builders.

---

## 3. Internal memory data flow

![figgy internal memory and render architecture](crates/renderer/assets/architecture-state-flow-en.png)

The renderer-owned registry is the persistent SSoT used by the browser wrapper.
Low-level native hosts may still supply `ChartDrawItem` directly. In both paths,
`ChartDrawItem` is prepare-only input; paint consumes only the owned token.

### Exact streaming and residency

![figgy exact streaming architecture](crates/renderer/assets/streaming-architecture-en.png)

The image shows the baseline **on-screen** streaming path; it does not yet show
the optional chart-local packed-view cache. The host owns replayable original data:
either stable TypedArrays or a `readRange` provider. The web facade handles
browser scheduling and requests only the ranges needed for the current job; it
does not own the stream cursor, chart state, or accumulated statistics. The
renderer owns `Config`, ordered series, source revisions, the cursor, cached
range summaries, and view-local residency admission. Automatic streaming never
promotes a connected whole-column closure into the global `ColumnPool`. It
draws through bounded GPU uploads and an offscreen accumulation surface, then
may retain the exact original rows needed by the view within the configured
working-set and total-GPU budgets. Both paths draw original
primitives, without LOD, sampling, or decimation. A resident chart and a
streamed chart can coexist on one page.

Streaming presents completed portions while the next ranges are supplied. An
unchanged completed revision reuses its visible result; decoration-only edits
keep the data cursor and accumulation. A view or physical-resolution change
replays the same source revision at the new transform. `job.cancel()` stops new
work and releases job-owned resources after submitted GPU work settles. A
narrower view may redraw from the packed GPU cache without rereading the source;
other view changes replay it. GPU point/line picking uses the packed rows when
available and returns original row indices without source replay. Other streamed
charts do not support immediate picking. Scaled PNG export uses replayable
original ranges in a separate GPU path, so the source must remain available.
See [the web streaming contract](crates/renderer/WASM.md#exact-streaming)
for the current API, support limits, and lifecycle details. Exact original-data
processing does not imply byte-identical antialiasing across GPU backends or
render-pass boundaries.

### Ownership and lifetime boundaries

`Renderer` owns the chart registry and GPU-side state: each chart's authoritative
`Config` and ordered `SeriesConfig`, the resident `ColumnPool`, nonresident
logical-source metadata in the 0.12.0 candidate, render/compute pipelines,
the shared picker pipeline bundle, at most one derived active-chart picker cache,
pending pool maintenance, bind groups, per-panel GPU resources such as
`ChartView` / `ChartStyle`, and the shared `Arc<wgpu::Device>` /
`Arc<wgpu::Queue>`. A `ChartId` is opaque and bound to its issuing renderer.
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

Renderer 0.9 keeps exact GPU picking chart-aware. Call
`enable_gpu_picking()` once, optionally call
`prepare_gpu_picking_for_chart(chart_id)` to make a chart first-pick-ready, and
submit through `pick_chart(chart_id, GpuPickRequest)` or
`WindowedRenderer::pick_chart_at`. The renderer derives axis transforms and the
data-area clip from its authoritative `Config`; the public low-level
`GpuPickEngine` surface from 0.7 is no longer exposed. Picking reads the GPU
column pool directly: there is no CPU point mirror and no internal `Mutex`.
Use `pick_chart_data` / `WindowedRenderer::pick_chart_data_at` for the tagged
point, histogram-bin, matrix-cell, and contour-level result.

Every mutation — `Renderer::prepare` and the export prepare path — runs behind
an `&mut self` boundary and does not introduce a shared lock inside the
renderer. `Renderer::paint_prepared` records through `&self` against an owned
`PreparedFrame` token, so host paint callbacks that only hand out shared access
need no wrapper lock either (`Renderer` is `Send + Sync`).

The token captures resolved pipelines, bind groups, buffers, panel geometry,
column allocation epochs, pool layout generation, target-pipeline generation,
and each captured `ChartView` content revision. A mismatch fails before
recording with `FiggyError::StalePreparedFrame`; the host prepares again on the
next frame. Arc/star scratch and automatic contour placement are immutable
exact-key results: equal compute inputs share one result, while changed
geometry, data generation, or placement inputs allocate a new result and never
overwrite the old one. Their shared GPU charges live as long as the cache or a
prepared token owns the corresponding handles; after the last Figgy owner
drops, retired accounting remains until the host reports queue submission with
`end_gpu_frame()`. Explicit contour placement is immutable too. This
guarantee does not replace host frame ordering: rewriting the same `ChartView`,
replacing a captured column, defragmenting the pool, or rebuilding target
pipelines deliberately makes the old token stale. Commands recorded from that
token must be submitted before one of those mutations.

For resident `add_column`, `ColumnSource` data is borrowed only during upload.
The long-lived records are the GPU-pool column and the scalar stats cached for
auto-fit (min / max / smallest-positive); source references and CPU-side
per-point geometry are not kept. Per-point geometry such as dashed-line and
constellation arc prefixes is derived from the GPU pool by compute scans.
Nonresident registration instead retains logical source metadata and bounded
GPU work; its host must keep the original range provider available for exact
replay. The renderer does not retain a full CPU copy of that source.

An in-flight `GpuPickTicket` owns its readback resources and an `Arc`-backed
identity mapping captured at submission. Later chart or pool mutations, and
even dropping the renderer, cannot remap that ticket's eventual
`source_id` / `series_id` result.

The browser public surface follows the same boundary. The `<figgy-chart>`
facade owns the shadow canvas, ready promise, rAF loop, ResizeObserver/DPR
handling, pointer mapping, async-operation busy gate, and id register/unregister
lifecycle. The raw `FiggyChart` wasm kernel remains available as an advanced
escape hatch.

Web cold-start and lifecycle contract:

| Surface | Contract |
|---|---|
| Raw `FiggyChart` | In a wasm browser, `create` / `create_with_progress` warm every render WGSL entry on the same `GPUDevice` through Promise-based `createRenderPipelineAsync`, discard the temporary JS pipelines, then submit and await the first empty-chart frame. Production renderer-owned optional render/style and arc/fit/picker/contour compute caches remain lazy. `prewarm_all_with_progress(callback)` publishes those actual wgpu caches with `{ scope, stage, phase }` progress; `prewarm_all()` performs the same work without a callback. `warm_up()` is a first-frame compatibility alias, not full prewarm. Creation does not enable the production picker: `prewarm_gpu_picking()` explicitly enables it and prepares the current chart, while retries and `pick_point` / `pick_data` reuse the same renderer-owned path and sticky activation error. |
| `<figgy-chart>` startup | The `web.create / first frame / finished` progress event and `figgy-ready` are published before background picker prewarm begins. A prewarm failure emits `figgy-error` with `operation: "prewarm_gpu_picking"` and `recoverable: true`; the fulfilled `ready` promise and rendering loop remain valid. |
| Async serialization | One generation+kernel operation token covers connect/create, `prewarm_all_with_progress` / `prewarm_all` and picker prewarm, export, `first_frame_ready` / `warm_up`, extent preparation, async fit, and pick. The facade's two full-prewarm methods pass through this existing generation-aware operation gate. While `busy`, rAF drawing and pointer/proxy kernel access do not enter wasm; only the latest resize and a pending pointer release are retained and applied after settlement. |
| Disconnect/reconnect | Disconnect invalidates the generation and cancels its rAF/observer. A kernel borrowed by an active operation is freed only after that operation settles. Its stale settlement cannot clear, resize, release, or free the new generation's kernel. |

Web mutation API contracts:

| API | Contract |
|---|---|
| `auto_fit_colorbar(padding)` | Fits the shared colorbar z axis to the upload-metadata union of every matrix value column. A chart without a colorbar is unchanged. |
| `set_colorbar_axis(json)` | Replaces the existing colorbar's complete `AxisOptions` SSoT, covering tick style/direction/length, inversion, tick-label style/offsets, and title options. |
| `set_colorbar_title(text)` | Sets the colorbar title and shows it; an empty string hides it. Fails when `Config.colorbar` is absent. |
| `set_contour_nice_levels(series_id, target_count, use_colormap_colors)` | Uses the colorbar axis tick rules to replace one contour series' explicit levels, optionally assigns color-map colors, records the result in series SSoT, and returns the resulting level count. |
| `series_draw_info(series_id)` | Reports `{ drawn_count, cols, rows, truncated }`. The raw wasm `FiggyChart` returns a JSON string; the `<figgy-chart>` facade parses it and returns the object. |
| `pick_data(x, y, max_distance_px)` | Asynchronously returns a tagged point/bin/cell/contour identity or `null`; raw wasm returns its JSON string or `undefined`, and the facade parses it. |
| `set_picked_points(json)` | Accepts a JSON string encoding `PickedPointsConfig` or `null`. It replaces only renderer-owned `Config.picked_points`; `null` clears the overlay. References retain `series_id`, optional `source_id`, and `point_index`, not copied point coordinates. |
| `set_picked_data(json)` | Accepts `DataSelectionsConfig` or `null` and replaces only `Config.picked_data`. Refs retain stable indices and provenance; current geometry stays in the GPU-backed chart SSoT. |
| `set_clear_color(r, g, b, a)` | Accepts linear RGBA components, clamps each to `0..1`, and schedules a surface redraw. Clear color is host/surface state and does not modify Config JSON or force an axis-raster refresh. |

These ownership rules support the data fidelity contract: renderer/web keep
source columns intact, and clipping, log-domain skips, NaN skips, and
antialiasing limits stay rendering decisions rather than data rewrites.

### Dashed-line arc scan (GPU)

The dash phase needs the cumulative pixel arc length at every point, which
depends on the live data→pixel transform. It is produced entirely on the GPU
for each distinct compute key; an exact key hit reuses the immutable result:

```
pool columns (x, y) ──┐                       Transform uniform (96 B write)
                      ▼                                   │
   seg_init           dst[i] = |px(pᵢ) − px(pᵢ₋₁)|   ◄────┘
   scan_block         256-block inclusive scans (Hillis–Steele, shared mem)
   scan_block/add     block-sum levels (dst → sums0 → sums1)
   carry chain        chunks of min(dispatch limit × 256, 256³) points run
                      sequentially; a 1-element carry buffer folds each
                      chunk's total into the next — n is bounded only by
                      pool memory, with no readback at any size
                      ▼
   arc prefix buffer ──► line pipeline vertex slots 4/5 (dash phase)
```

The compute encoder is submitted before the host's render pass, so queue
order sequences it under every embedding (winit / egui / iced / web) without
API changes. The exact key includes pool layout generation, x/y offsets and
allocation epochs, length, every geometry-transform bit read by the compute
shader, and optional star pitch. Each series retains its eight most recent
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
| renderer `raster` revision | Config, series, selection, or font changes conservatively require `refresh_axis` before drawing |
| host `view_dirty` | Surface/DPR preview geometry changed; refresh raster and draw |
| host `redraw_pending` | Host-only surface state such as clear color changed; draw without duplicating chart state |

Browser frame flow:

```rust
renderer.sync_external_invalidations()?;
let stamp = renderer.chart_render_stamp(chart_id)?;
let draw = stamp.needs_draw_since(last_presented_stamp.as_ref());
let raster = stamp.needs_raster_since(last_presented_stamp.as_ref());

if !draw && !view_dirty && !redraw_pending {
    process_maintenance_without_surface_if_needed()?;
    return Ok(());
}
if raster || view_dirty {
    renderer.refresh_axis_with_selection(&mut view, &display_chart, rect, &boxes)?;
}
renderer.draw(clear, &items)?;
last_presented_stamp = Some(renderer.chart_render_stamp(chart_id)?);
view_dirty = false;
redraw_pending = false;
```

The last-presented stamp and host flags advance only after a successful draw, so
a failed visual frame is retried. A clean web rAF skips the entire GPU surface
path. A resident redraw records data primitives again from the pool; that path
has no data-layer image cache. Candidate nonresident rendering instead
keeps a bounded GPU accumulation surface for partial display and reuses an
unchanged completed revision. Neither path applies data LOD, sampling, or
decimation.

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
- GPU shader: `mix(v, log10(v), is_log)` — branch-free ALU. Non-positive data samples become NaN/ignored by the data path, not a config validation failure.

### Export pipeline

```
export_panel_rgba_async(chart, &[SeriesConfig], scale).await:
    scale ← clamp_export_scale(scale)         // [MIN_EXPORT_SCALE, MAX_EXPORT_SCALE]
    chart.config().scaled(scale)               // every pixel dim scaled proportionally
        ↓
    temp ChartView (scaled axis textures)
    temp ChartStyles ← create_style_for_series_scaled(cfg, scale) per cfg
        ↓
    offscreen wgpu::Texture (fixed Rgba8Unorm, COPY_SRC, transparent clear)
    paint(items) — same compositing order (grid → data → decoration)
        ↓
    copy_texture_to_buffer in ROW CHUNKS (256-byte aligned padding; chunk
    height adapts to the device's max buffer size, so huge exports survive)
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

---

## License / fonts

Bundled font: Liberation Sans (SIL OFL 1.1) — `crates/renderer/fonts/LICENSE-LiberationSans.txt`. Hosts can register additional fonts at runtime (`register_font` on wasm, `text_render::register_font_bytes` on native). Byte-for-byte duplicate registration is idempotent: the registry stores the file once, reuses resolved face backing by face id, and does not advance the global font generation.

---

<a id="한국어-문서"></a>

# figgy (한국어 문서)

Rust 과학 차트 라이브러리. **CPU 라스터 (축 / 라벨 / 그리드 — tiny-skia + swash) + GPU wgpu (대량 데이터) 하이브리드** 렌더링.
egui / winit / 기타 wgpu 30 호스트에 임베드할 수 있다.

> 워크스페이스 루트 README. crate 3개로 구성:
> **`crates/model`** — 순수 차트 모델이자 스키마 권위: 옵션/데이터 SSoT(`Config`, `SeriesConfig`), 리치텍스트/범례 문서 모델, 상호작용 정책(`Selectable`/`Draggable`/`Resizable`, `HitMap`, 단일 이동 경로 `Config::nudge`), 프리셋(`AxisPreset`, `ColorCycle`). 의존성 0, `serde` 는 선택 피쳐.
> **`crates/renderer`** — 아래에서 문서화하는 wgpu + CPU 라스터 장치. 지속 chart registry(`ChartId` → `Config`, 순서 있는 `SeriesConfig`, selection, checked revision), 상주 `ColumnPool`, 0.12.0 후보의 비상주 논리 소스 metadata, picker pipeline bundle과 파생된 단일 active-chart registry cache, pending GPU-pool maintenance를 소유한다. `model`을 의존하고 public 모듈을 re-export한다.
> **`crates/web`** — 브라우저 패키지(`figgy`): public `<figgy-chart>` Custom Element facade와 advanced escape hatch로 남는 raw `FiggyChart` wasm kernel. facade가 shadow canvas, ready promise/event 수명주기, rAF loop, ResizeObserver/DPR 처리, pointer mapping, async operation busy gate, id 기반 등록 metadata, UI 파생 label/style/extent, Promise 변환을 소유한다. picker, pool, chart, maintenance 권위는 `Renderer`에 남는다. 브라우저 I/O: [WASM.md](crates/renderer/WASM.md) · Config JSON 스키마: [SCHEMA.md](crates/web/SCHEMA.md). 빌드 산출물(`crates/web/pkg/`)은 gitignore — `npx wasm-pack build crates/web --release --target web` 로 빌드.
> **웹 스튜디오** — [figgyplot.com](https://figgyplot.com/) 에 공개 웹 편집기가 있다. 브라우저 안에서 로컬 차트 데이터를 처리하고, CSV/TSV/Excel import, `.figgy` 프로젝트 열기, 같은 wasm/WebGPU 표면 기반 PNG export를 제공한다.

## 공개 후보 — renderer 0.12.0 / figgy 0.10.0

이번 후보에는 렌더러가 소유하는 원본 데이터 스트리밍과 웹의 `render_chart()`
작업 API가 들어간다. 웹 facade는 필요한 원본 구간만 요청해 실행 일정과 진행
상태를 연결한다. 지원되는 정밀 점·실선·에러바 차트에서는 렌더러가 현재 화면에
필요한 원본 행만 차트별 GPU 캐시에 유지할 수 있다. 연결된 컬럼 전체를 자동으로
상주 풀에 승격하지 않는다. 완료된 revision은 재사용하고, 더 넓은 뷰나 배율
출력에는 동일 원본을 다시 공급할 수 있다. LOD나 다운샘플링은 없다. 지원 범위와 제외 항목은
[WASM.md](crates/renderer/WASM.md)에 정리했다. 이 소스 후보가 공개 웹
Studio에 적용됐다는 뜻은 아니다. 공개 검증과 배포가 끝나기 전의 실제 공개
버전은 0.11.0 / 0.9.1이다.

이전 renderer 0.11.0 / figgy 0.9.1 릴리스의 변경 내용:

1픽셀 미만 히스토그램 bin은 GPU에서 픽셀 열별 최댓값을 골라 0까지 채운다.
선 두께와 알파가 모두 양수면 영역 전체를 선 색으로, 아니면 면 색으로 채운다.
원본 컬럼과 웹 API 형식은 바뀌지 않는다. Rust `ColumnBarLayer`에는 `envelope`
필드가 추가되어 외부 struct literal은 수정해야 한다(수동 레이어에 envelope가
없으면 `None`). 데모에는 bin 개수 슬라이더와 외곽선 토글이 있다.
`npx serve crates/web -l 8142` 실행 후 `http://localhost:8142/`에서 확인할 수 있다.

renderer 0.10.0 / figgy 0.9.0 릴리스에 포함된 기능은 다음과 같다.

- **히스토그램과 행렬 필드를 GPU 일급 시리즈로 제공한다.** `Histogram`, `Heatmap`, `Contour`, `ContourFill`이 선언된 matrix lattice와 colour-map SSoT를 공유한다. heatmap은 flat/interpolated shading을, contour는 최대 1024 level을 지원하며 라벨 위치에서는 실제 등고선을 끊는다. 필드 자동 맞춤은 sample centre가 아니라 실제 렌더링되는 cell 경계를 사용한다.
- **필드 선택과 스타일은 stable identity를 사용한다.** `pick_data`는 point, histogram bin, matrix cell, contour level의 tagged ref를 반환하며 `Config.picked_data`로 다시 표시할 수 있다. histogram은 막대 폭, 외곽선 색/두께, 개별 bin override를 제공한다. contour label 글자색·배경·숫자 형식은 level 선 색과 독립적이다. colorbar는 tick, label, title, reverse와 포인터를 따르는 resize handle을 포함한 전체 `AxisOptions`를 노출한다.
- **GPU 범위와 브라우저 시작 계약을 정확하고 관찰 가능하게 만들었다.** hi/lo field 좌표 연산과 범위 reduction은 GPU에 남고, 축 SSoT에 기록되는 범위는 실제 draw가 사용하는 값과 같다. 브라우저 시작은 contour-label width attribute를 포함한 모든 render entry를 검증하며, `prewarm_all_with_progress` / `prewarm_all`로 renderer-owned lazy cache를 명시적으로 게시할 수 있다.
- **GPU 메모리를 하나의 renderer budget으로 제한한다.** pool 저장소, staging, export, picking, contour placement와 기타 pool 밖 allocation을 device-aware limit에 함께 부과하고, 검사되지 않은 할당 전에 renderer error로 실패한다.

이 저장소가 지원되는 공개 source distribution이며 crate는 crates.io에 배포하지 않는다. 공개 Git revision을 고정한 소비자는 lockfile을 갱신하고 wasm package를 다시 빌드해야 한다. 브라우저 lifecycle/API는 [WASM.md](crates/renderer/WASM.md), 전체 JSON 계약은 [SCHEMA.md](crates/web/SCHEMA.md)를 참고한다.

- **상주 GPU columnar pool**: 상주 경로에 들어간 컬럼은 하나의 GPU buffer를 공유하고, first-fit 할당과 단편화 시 핑퐁 defrag를 사용한다. `HiLoColumnSource`로 올린 논리값은 f32 hi/lo 쌍으로 저장해 timestamp 크기의 offset도 GPU에서 보존한다. 업로드 시 auto-fit 용 스칼라 통계(min / max / 최소 양수)를 캐싱하고, 점선 호장 prefix 같은 per-point 지오메트리는 컴퓨트 스캔(`line_arc.wgsl`)이 제자리에서 계산한다.
- **비상주 렌더링(0.10.0 공개 후보)**: 재공급 가능한 컬럼은 논리 ID·길이·인코딩·revision만 등록하고 전체 데이터를 GPU pool에 보관하지 않는다. 렌더러가 원본의 필요한 구간을 제한된 크기로 요청해 빠짐없이 누적해서 그리며, 화면이나 출력 조건이 바뀌면 호스트가 같은 원본을 재공급한다. 데이터 축소·다운샘플링은 하지 않는다. 입장 판단·지원 스타일·취소·완료 후 조회 계약은 [WASM.md](crates/renderer/WASM.md)에 별도로 적었다.
- **분리 합성**: grid → data → axis/label/legend 순으로 합성 → 그리드가 데이터를 가리지 않음. axis raster는 `Grid` / `Decoration` 분리 레이어가 기본이고, `AxisLayerKind::All`은 legacy 단일 패스 helper로 남아 있음.
- **데이터 무왜곡 계약**: renderer/web은 model 계약을 소비하며 원본 좌표, provenance, 축↔데이터 대응을 호스트 동의 없이 조용히 바꾸지 않는다. 명시적 clipping, log-domain skip, NaN skip, antialiasing 한계는 데이터 재작성 아닌 렌더링 계약이다.
- **헤드리스 PNG export**: 임의 DPI 로 GPU offscreen 라스터 → 메모리 RGBA / PNG 바이트 반환 (async 우선, native 는 blocking 래퍼 제공).
- **상호작용 레이어 (opt-in)**: 히트테스트, 선택 박스, 드래그(축은 수직 방향 제약 + 분리 축 `line_offset`), 데이터 영역 PPT 식 8핸들 리사이즈 — 정책은 전부 `model`, 호스트가 포인터 이벤트를 넣을 때만 동작.
- **데이터 피킹 (opt-in)**: `pick_point`는 기존 point/line 호환 계약을 유지하고, `pick_data`는 histogram bin, canonical matrix cell, contour level의 tagged identity를 추가한다. 상주 경로는 draw와 같은 transform·pool·style·lattice·level table을 읽는 GPU shader에서 판정한다. 완료된 차트별 패킹 뷰는 원본을 다시 읽지 않고 GPU에서 point/line을 피킹하며 원본 행 인덱스를 반환한다. 그 밖의 비상주 스트림은 `null`을 반환하고 저수준 WASM 스트림 피킹 진입점은 즉시 거절한다. 이미 알고 있는 stable ref는 `Config.picked_data` 또는 `Config.picked_points`로 지정해 필요한 행만 읽어 강조 표시할 수 있다.
- **점별 스타일 매핑 (opt-in)**: precise scatter는 `point_style_table` / `point_style_index_column` / `point_style_overrides`를, precise errorbar는 독립적인 `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`를 바인딩할 수 있다. styled mode는 자체 visual shader를 사용하며 이 매핑을 무시한다.
- **리치텍스트 일원화**: 제목·틱 라벨·범례가 한 엔진 공유 — 세그먼트별 bold/italic/밑줄/첨자/그리스, 세그먼트별 색·크기 오버라이드, `'\n'` 줄바꿈, `'\t'` 표 열, 고정폭 범례 심볼 필드.
- **손그림 스케치 모드 (opt-in)**: `draw_style: { mode: "sketch", amplitude_px, wavelength_px, seed }` 한 필드로 차트 전체를 xkcd 풍으로 — 축/틱/그리드/범례는 CPU 라스터에서, 라인의 흔들림/점선 위상은 호장 스캔 기반 GPU 변형으로, 마커/에러바는 전용 GPU 변형으로 처리되고, 차트 텍스트는 번들 손글씨 폰트(Comic Neue, OFL)로 자동 전환된다(글리프 없는 문자는 문자 단위 폴백 — CJK는 등록 폰트 유지). 시드 기반 결정적, 점선과 합성 가능, 필드가 없으면 정밀 경로가 한 바이트도 달라지지 않는다.
- **은하수(milkyway) 모드 (opt-in)**: `draw_style: { mode: "milkyway", ... }`는 차트를 천체사진처럼 렌더링한다. 라인은 시리즈색 성운 리본 위 별 사슬, scatter marker는 고리 행성, errorbar는 심우주 배경 위 양극 제트가 된다.
- **성좌(constellation) 모드 (opt-in)**: `draw_style: { mode: "constellation", ... }`는 `ScatterLine` series만 지원한다. scatter 데이터 위치에 PSF 별을 놓고 반투명 선으로 연결한다. 파라미터 범위는 기계가 읽는 `draw_style_param_specs` metadata로 제공한다.
- **단일 wgpu 메이저 (30)**: renderer와 활성 egui 통합은 wgpu 30을 공유한다. iced 0.14는 아직 wgpu 27 타입을 노출하므로 보존된 iced 통합 소스는 빌드 대상에 넣지 않는다.
- **WebAssembly 지원**: 순수 Rust 라스터 스택(tiny-skia + fontdb + swash), async 초기화/export, 런타임 폰트 등록(`register_font`) 으로 CJK·커스텀 패밀리 지원.
- **관찰 가능한 웹 초기화**: wasm 브라우저의 `create` / `create_with_progress`는 동일 `GPUDevice`의 모든 render WGSL entry를 Promise 기반 `createRenderPipelineAsync`로 데우고 임시 JS pipeline을 버린 뒤 빈 차트 첫 frame 완료까지 기다린다. Renderer-owned optional render/style과 arc/fit/picker/contour compute cache는 최초 사용 또는 명시적 `prewarm_all_with_progress` / `prewarm_all`까지 lazy다. `<figgy-chart>`는 첫 성공 frame과 `figgy-ready`를 공개한 뒤 renderer-owned GPU picker를 background prewarm한다.

### 렌더링 스타일 미리보기

같은 growth-response 데이터를 네 가지 차트 스타일로 렌더링한 비교:

<table>
  <tr>
    <td width="50%"><strong>정밀(Precise)</strong><br><img src="crates/renderer/assets/style-growth-response-precise.png" alt="정밀 스타일 growth-response 차트" width="420"></td>
    <td width="50%"><strong>스케치(Sketch)</strong><br><img src="crates/renderer/assets/style-growth-response-sketch.png" alt="스케치 스타일 growth-response 차트" width="420"></td>
  </tr>
  <tr>
    <td width="50%"><strong>은하수(Milkyway)</strong><br><img src="crates/renderer/assets/style-growth-response-milkyway.png" alt="은하수 스타일 growth-response 차트" width="420"></td>
    <td width="50%"><strong>성좌(Constellation)</strong><br><img src="crates/renderer/assets/style-growth-response-constellation.png" alt="성좌 스타일 growth-response 차트" width="420"></td>
  </tr>
</table>

---

## 1. 사용법

### 의존성 추가

```toml
[dependencies]
renderer = { path = "crates/renderer" }   # 또는 공개 Git source — 후보 0.12.0, crates.io 미배포.
wgpu     = "30"
```

라이브러리 자체는 winit / egui / iced 어느 것에도 의존하지 않습니다. 사용하는 호스트만 추가:

```toml
# winit standalone
winit = "0.30"

# egui 임베드
eframe    = { version = "0.36", default-features = false, features = ["wgpu"] }
egui      = "0.36"
egui-wgpu = "0.36"
```

iced 0.14는 아직 wgpu 27을 사용한다. 따라서 iced가 wgpu 30 호환 버전을
배포하기 전까지 device/queue/render pass 직접 공유 통합은 비활성 상태다.

### 가장 짧은 standalone 예 (winit + figgy 단독 wgpu)

```rust
use std::sync::Arc;
use renderer::{
    Chart, ChartDrawItem, DataLineStyleConfig, DataRenderType, Renderer, Series, SeriesConfig,
    color::Color, default, layout::{ChartArea, Rect}, line::LineStylePreset,
};

let window = Arc::new(event_loop.create_window(attrs).unwrap());
let size = window.inner_size();

// 한 줄 셋업 — instance/adapter/device/queue/surface/swap chain 모두 figgy 가 소유.
let mut renderer = Renderer::for_window(
    Arc::clone(&window),
    (size.width, size.height),
    16 * 1024 * 1024,   // GPU column pool 16 MiB
).unwrap();

// renderer.add_column 은 `&dyn ColumnSource` 받음.
// 본인 데이터 타입에 trait 구현 (아래 `ColumnSource` 섹션 참조) — Vec, ndarray,
// polars Series, mmap 등 어떤 출처든 zero-copy 업로드. 빌트인 `Column<f64>` 도 사용 가능.
let xs: Vec<f64> = (0..1024).map(|i| i as f64 * 0.01).collect();
let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
renderer.add_column("x", &my_source_for(0, xs)).unwrap();   // your type : ColumnSource
renderer.add_column("y", &my_source_for(1, ys)).unwrap();

// Chart — 빌더 패턴.
let mut config = default::default_config();
config.chart_area = ChartArea(Rect { x:8, y:8, width: size.width - 16, height: size.height - 16 });
let mut chart = Chart::new(config)
    .with_title("Sine")
    .with_x_title("x")
    .with_y_title("sin(x)");
chart.auto_fit_x(renderer.pool(), "x", 0.05).unwrap();
chart.auto_fit_y(renderer.pool(), "y", 0.10).unwrap();

// 시리즈 = SeriesConfig (선언) + ChartStyle (그 선언에서 자동 빌드된 GPU 스타일).
let cfg = SeriesConfig {
    series_id: "sin".into(), label: None,
    source_id: None,
    x_column: "x".into(), y_column: "y".into(),
    render_type: DataRenderType::Line {
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::from_rgb8(20, 110, 230),
            line_width: 2.0,
        },
    },
};
let style = renderer.create_style_for_series(&cfg);            // SeriesConfig → ChartStyle
let view  = renderer.create_chart_view(&chart, chart.config().chart_area.0).unwrap();

// frame loop:
let series = [Series { config: &cfg, style: &style }];
let items  = [ChartDrawItem {
    view: &view,
    chart_config: chart.config(),
    series: &series,
}];
renderer.draw(Color::WHITE, &items).unwrap();   // surface frame 획득 → prepare → encoder → pass → paint_prepared → submit → present
```

### `ColumnSource` — 데이터 어댑터 trait

`Renderer::add_column` 의 시그니처는 `&dyn ColumnSource` 입니다 — 어떤 데이터 컨테이너든 본인 타입에 trait 구현하면 GPU pool 에 zero-copy 로 들어갑니다 (`Vec` 중간 alloc 0). source가 GPU pair 기록과 최소 양수 통계를 같은 pass에서 수행하고, renderer는 wgpu 30의 write-only mapped byte를 다시 읽지 않습니다. `min` / `max`의 source-level 의미는 그대로입니다. scalar 최소 양수는 실제 업로드된 `(value as f32, 0)`, hi-lo 최소 양수는 기록된 `hi as f64 + lo as f64` 기준이며 finite positive 값만 포함합니다.

`add_column` / `add_hilo_column` 은 새 id 등록용이다. 기존 id를 원자적으로
교체할 때는 `upsert_column` / `upsert_hilo_column` 을 사용한다. `Renderer`는
새 pool 후보, 영향받는 chart revision, active picker transition, maintenance
상태를 전부 준비한 뒤 한꺼번에 공개한다. 준비 단계가 오류를 반환하면 기존
권위 상태가 유지된다. 추가적인 host-owned 파생 상태가 있는 통합은
`begin_upsert_*` guard의 provisional pool을 보고 그 파생 상태를 준비한 다음
실패하지 않고 allocation도 하지 않는 `commit`을 호출할 수 있다. host가
picker를 별도로 재구축하지는 않는다.

```rust
pub trait ColumnSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }  // 디폴트 제공
    fn min(&self) -> f64;
    fn max(&self) -> f64;

    /// source compatibility를 위해 유지되는 legacy scalar encoder.
    /// 호출자는 `dst.len() == self.len() * 4` 보장. null → `f32::NAN`.
    fn write_f32_le_into(&self, dst: &mut [u8]);

    /// `(value as f32, 0)` pair와 통계를 같은 pass에서 기록.
    fn write_f32_pair_le_into_with_stats(
        &self,
        writer: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats;
}
```

**빌트인 구현체**: `Column<f64>`, `Column<f32>`, `Column<Option<f64>>` (null → NaN).

custom trait 구현은 fused method를 반드시 구현해야 한다. 불완전한 migration은
upload 중 런타임 오류가 아니라 컴파일 오류로 드러난다. byte readback이나
부정확한 silent fallback은 없다. `HiLoColumnSource`도 같은 이름의 fused
method에서 `(hi, lo)` 기록과 reconstructed 통계를 함께 반환해야 한다.

**사용자 정의 — 시계열 / DataFrame / mmap / FFI 데이터 등 어떤 출처든**:

```rust
struct MyTimeSeries {
    samples: Vec<f64>,    // 또는 Arc<[f64]>, ndarray::ArrayView, polars::Series, ...
    cached_min: f64,
    cached_max: f64,
}

impl renderer::ColumnSource for MyTimeSeries {
    fn len(&self) -> usize { self.samples.len() }
    fn min(&self) -> f64 { self.cached_min }
    fn max(&self) -> f64 { self.cached_max }
    fn write_f32_le_into(&self, dst: &mut [u8]) {
        debug_assert_eq!(dst.len(), self.samples.len() * 4);
        for (i, &v) in self.samples.iter().enumerate() {
            dst[i*4..i*4+4].copy_from_slice(&(v as f32).to_le_bytes());
        }
    }
    fn write_f32_pair_le_into_with_stats(
        &self,
        mut writer: renderer::ColumnPairWriter<'_>,
    ) -> renderer::ColumnUploadStats {
        debug_assert_eq!(writer.len(), self.samples.len());
        let mut min_positive: Option<f64> = None;
        for (index, &sample) in self.samples.iter().enumerate() {
            let value = sample as f32;
            writer.write_pair(index, value, 0.0);
            let value = value as f64;
            if value.is_finite() && value > 0.0
                && min_positive.map_or(true, |current| value < current)
            {
                min_positive = Some(value);
            }
        }
        renderer::ColumnUploadStats { min_positive }
    }
}

renderer.add_column("temperature", &my_series)?;   // ↘ mapped staging memory 에 직접 write, Vec 0
```

`f32` 네이티브 컨테이너도 같은 fused 경로를 사용한다. 값을 순회하며
`writer.write_pair(index, value, 0.0)`를 호출한다. `ColumnPairWriter`는 mapped
byte 대신 논리적인 pair 쓰기만 공개하므로, wgpu나 `dst.copy_from_slice(...)`
우회 없이도 active pool upload에 중간 allocation이 생기지 않는다.

### 네이티브 example — 사인 / RC / cross-section

```bash
cargo run -p renderer --example winit_simple
cargo run -p renderer --example egui_embed --features egui_demo
```

각 example 은:
- 3 panel grid (그리드 옵션 다름: 끔 / major / major+minor 점선)
- RC panel 은 충전 + 방전 2 시리즈
- 라인 두께 1 / 2 / 3.5 px 차등
- 범례 표시
- DPI 입력 + Save PNG 버튼 (egui) 또는 `S` 키 (winit) 으로 panel 별 PNG 메모리 export → `/tmp/figgy_*_panel_{i}.png`

### 브라우저 timestamp 축 데모

`crates/web/timestamp-demo.html` 은 absolute Unix time 값을
`register_column_f64(Float64Array)` 로 처음 올리고
`update_register_column_f64` 로 명시 교체하는 브라우저 timestamp 경로를
확인하는 데모다. 시간 범위, 데이터 단위, 시간대, 소수 초 정책, 라벨 패턴,
차트 폭, export scale 을 바꾸면서 x 축이 `LabelFormat::Timestamp` +
`AutoCalendar` 로 겹치지 않는 라벨을 고르는지 볼 수 있다.

```bash
npx wasm-pack build crates/web --release --target web
cd crates/web
python -m http.server 8142 --bind 127.0.0.1
# http://127.0.0.1:8142/timestamp-demo.html 열기
```

### 라이브 SSoT lab — 풀 스케일에서 본 분리 API

```bash
cargo run --release -p renderer --example ssot_lab --features egui_demo
```

2×2 grid, panel 마다 draw style 하나씩 (Precise dashed / Sketch / Milkyway /
Constellation), 네 시리즈가 공유 `x` 풀 컬럼 하나를 함께 읽는다. 사이드바가
SSoT 를 라이브로 편집한다 — x 연동된 열 쌍별 pan 방향, window 폭, 시리즈당
포인트 밀도 최대 3M (합계 12M, 5 컬럼). 모든 편집이 `Renderer::prepare` →
`Renderer::paint_prepared` 로 흐르며 `Mutex` 도 frame 별 `update_transform`
도 없다. 상태창의 `frames skipped` 가 0 으로 유지되어, 라이브 편집 중에도
토큰이 stale 되지 않음을 증명한다.

### egui 통합 패턴 (요약)

프레임이 호스트 콜백 형태에 맞게 둘로 나뉜다: 모든 변경은
`Renderer::prepare` (`&mut self`), 순수 커맨드 기록은
`Renderer::paint_prepared` (`&self`) — paint 콜백에서 렌더러를 `Mutex` 로
감쌀 필요가 없다:

```rust
// CallbackResources 에 FiggyState 그대로 저장 — Mutex 없음
struct FiggyState { renderer: renderer::Renderer, panels: Vec<PanelEntry> }
struct PanelEntry { /* chart, view, … */ prepared: Option<renderer::PreparedFrame> }
struct FiggyCallback { panel_idx: usize /* panel 당 콜백 하나 */ }

impl egui_wgpu::CallbackTrait for FiggyCallback {
    fn prepare(&self, _device, _queue, _screen, _enc, resources) -> Vec<...> {
        let state = resources.get_mut::<FiggyState>().unwrap();
        // dirty 처리: refresh_axis (raster). frame 별 update_transform 은 불필요 —
        // prepare 가 transform uniform 을 직접 쓴다.
        let prepared = state.renderer.prepare(&items).unwrap();
        // 토큰은 panel 별로 저장 — egui 는 모든 콜백의 prepare 를 먼저 돌린 뒤
        // paint 를 돌리므로, 공유 토큰 하나면 뒤 panel 의 prepare 가 덮어쓴다
        state.panels[self.panel_idx].prepared = Some(prepared);
        Vec::new()
    }
    fn paint(&self, info, render_pass, resources) {
        let state = resources.get::<FiggyState>().unwrap();
        let prepared = state.panels[self.panel_idx].prepared.as_ref().unwrap();
        let target = (info.screen_size_px[0], info.screen_size_px[1]);
        state.renderer.paint_prepared(render_pass, target, prepared).unwrap();
    }
}
```

호스트가 그 frame의 모든 command buffer를 submit한 뒤
`renderer.end_gpu_frame()`을 정확히 한 번 호출한다. egui처럼 callback을
스케줄링하는 host는 이전 frame이 submit된 것이 확실한 다음 frame의 첫 prepare
전에 호출해도 된다. panel별 prepare마다 호출하면 아직 submit되지 않은 다른
panel 자원의 retirement를 너무 일찍 지우므로 금지한다.

renderer가 submit까지 소유하는 경로(`WindowedRenderer::draw*`, panel export)는
이 경계를 내부에서 알린다. 외부 pass 기록과 이 경로를 함께 쓰는 host는 먼저
기록해 둔 command buffer를 모두 submit해야 한다. wgpu command buffer는 opaque라
renderer가 특정 host의 미제출 참조만 식별해 선택적으로 retire할 수 없다.

`paint_prepared` 는 반복 가능하다(같은 토큰을 여러 pass 에 기록 가능).
`PreparedFrame` 이 resolve된 draw input을 소유하므로 paint에서 `items`를
재구성하거나 다시 전달하지 않는다. 두 단계 사이에 캡처된 renderer 자원이
바뀌면 아무것도 기록하지 않고 `FiggyError::StalePreparedFrame` 을 반환한다
— 다음 frame 에 새로 `prepare` 하면 복구된다. automatic contour label도
panel/item + series occurrence 단위로 같은 소유권
규칙을 따른다. atlas와 cell table은 불변 cache 자원으로 공유하고, 각
서로 다른 dispatch 입력은 params, transform, candidate, anchor, indirect
args, compute bind group, GPU charge를 함께 소유하는 별도의 불변 placement
결과를 만든다. 정확히 같은 입력 key만 결과를 재사용하고, token이 drop된 뒤에도
다른 입력으로 기존 결과를 덮지 않는다. host command buffer가 제출 전의 옛 GPU
handle을 보유할 수 있기 때문이다. arc/star compute 결과도 같은 exact-key 규칙을
따르고 explicit anchor는 불변 snapshot이다. paint는 series cache를 다시 조회하지
않는다. token으로 기록한 command buffer는 같은 `ChartView`를 다시 prepare하거나
`refresh_axis`/`update_transform`하기 전에 제출해야 한다. 렌더러를 frame 동안
단독 소유하는 호스트(winit 루프, wasm 래퍼)는 두 단계를 연달아 실행하는 원샷
`Renderer::paint(&mut self, …)`
facade 를 그대로 쓰면 된다.

자세한 건 [examples/egui_embed.rs](crates/renderer/examples/egui_embed.rs).

### iced 통합 상태

보존된 [iced 통합 소스](crates/renderer/unsupported/iced_embed_wgpu27.rs)는
`prepare` / `paint_prepared` 소유권 패턴을 기록하지만 iced 0.14가 wgpu 27에
머무는 동안 빌드 대상은 비활성이다. 서로 다른 wgpu 메이저의 device, queue,
render pass 타입은 공유할 수 없다.

### PNG export (메모리 only — 저장은 caller)

```rust
let bytes = renderer
    .export_panel_png_bytes_async(&chart, &series_configs, scale)
    .await?;
std::fs::write("/tmp/out.png", &bytes)?;          // 또는 clipboard / network 등 자유.

// RGBA 만 필요하면:
let img = renderer
    .export_panel_rgba_async(&chart, &series_configs, scale)
    .await?;
// img.width, img.height, img.rgba (straight alpha, 길이 = w * h * 4)
```

native 전용 blocking convenience wrapper는 `_async` 없는 같은 이름을 쓴다.
`scale` 한계: `renderer::MIN_EXPORT_SCALE` (0.25) ~ `renderer::MAX_EXPORT_SCALE` (8.0) 자동 clamp.
`renderer::dpi_to_scale(dpi)` 로 표준 DPI(96) 기준 변환.

스케일 시 모든 픽셀 dim (폰트 / 선 / 마진 / 그리드 / 범례) 비례 확대 → 시각적 동치, 픽셀만 더 촘촘.

---

## 2. Config 구조체 필드 레퍼런스

```rust
pub struct Config {
    pub chart_area: ChartArea,           // 패널 픽셀 영역 (호스트 viewport 안)
    pub top_x: AxisOptions,              // 4 변 축 — 디폴트는 top/right 라벨/타이틀 비활성
    pub bottom_x: AxisOptions,
    pub left_y: AxisOptions,
    pub right_y: AxisOptions,
    pub chart_title: ChartTitleOptions,
    pub grid: GridOptions,
    pub legend: Legend,
    pub picked_points: Option<PickedPointsConfig>,
    pub picked_data: Option<DataSelectionsConfig>,
    pub colorbar: Option<ColorBarOptions>,   // the colourbar AND the chart's z scale
    pub draw_style: DrawStyle,
}
```

### `ChartArea` / `Rect`
| 필드 | 타입 | 의미 |
|---|---|---|
| `x, y` | u32 | 호스트 surface 좌상단 기준 패널 픽셀 위치 |
| `width, height` | u32 | 패널 픽셀 크기. 0 이면 live raster 실패 (`InvalidChartArea`). export chart area도 0이 되지 않게 호출자가 보장해야 하며, 현재 1px clamp는 호환 guard이고 추후 명시 오류로 바뀔 수 있음 |

### `AxisOptions` (top_x / bottom_x / left_y / right_y)
| 필드 | 타입 | 의미 |
|---|---|---|
| `scale` | `AxisScale` | `Linear` 또는 `Logarithmic` (log10) |
| `min, max` | f64 | 데이터 공간 범위. log scale에서는 양수 bound를 그대로 쓰고, 수동으로 들어온 0 이하/비정상 bound는 렌더러/축 경로에서 `1e-12`로 guard한다. 0 이하 데이터 샘플은 전체 range 오류가 아니라 skip/NaN 처리된다 |
| `major_spacing` | f64 | linear: 데이터 단위, log: decade 단위 (1, 2, …) |
| `minor_count` | usize | major 사이 minor 개수 (linear) 또는 decade 내 2..9 (log 시 8 추천) |
| `inverted` | bool | 축의 시각 방향을 반전한다. tick/grid 위치, 데이터 렌더링, picking 이 모두 같은 반전 mapping을 사용하며 `min`/`max`는 데이터 공간 bound로 유지된다 |
| `label_style` | `LabelStyle` | 눈금 라벨 스타일 |
| `tick` | `TickVisibility` | `None / Outside / Inside / Both` |
| `title_option` | `AxisTitleOptions` | 축 타이틀 텍스트 / 가시성 / 오프셋 |
| `out_margin` | f32 | 축 바깥쪽 (라벨+타이틀 band) 픽셀 마진 |
| `line_visible / color / width / style` | mixed | 축 선 외형. CPU raster stroke는 최소 1px로 floor되어 sub-pixel 폭이 사라지지 않음 |
| `line_offset` | f32 | 분리 축 오프셋: 데이터 영역은 그대로 두고 축 chrome(선/틱/라벨)만 수직 방향으로 평행이동. 레이아웃 비기여 — 드래그 시스템의 축 이동이 여기에 기록됨 |
| `major_tick_length / minor_tick_length` | f32 | tick 길이 (px) |

### `LabelStyle`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | 라벨 표시 여부 (overall) |
| `color` | `Color` | 라벨 색 |
| `font_size` | f32 | px |
| `label_visible` | bool | 숫자 라벨 자체 표시 여부 (visible 과 별개로 axis 자체는 켜고 라벨만 끄기) |
| `label_font` | String | 폰트 패밀리. 빈 문자열 → 번들 Liberation Sans |
| `label_offset_x / y` | f32 | nudge용 미세 오프셋 (px) |
| `format` | `LabelFormat` | `Decimal / Power / Scientific / Timestamp`. `Timestamp`는 linear axis에서 숫자 좌표를 Unix epoch 시간으로 해석하고 calendar-aware tick을 사용할 수 있음 |
| `significant_digits` | u8 | 유효 숫자 |

`LabelFormat::Timestamp`는 데이터 좌표를 계속 숫자로 유지한다. 기본값은 UTC
Unix seconds + `AutoCalendar` tick 계획이며, JS timestamp에는 `unit =
Milliseconds`, 한국 시간 같은 고정 오프셋에는 `FixedOffsetMinutes(540)`을 쓴다.
`AutoCalendar`는 tick label 폭을 측정해서 label이 겹치지 않도록 calendar step을
자동으로 성글게 만든다. 고해상도 absolute Unix timestamp는 native에서
`Renderer::add_hilo_column`, web에서 `register_column_f64` /
`update_register_column_f64`(`Float64Array`)를 사용하면 GPU에서
`(hi: f32, lo: f32)` pair로 보존된다.
[crates/web/timestamp-demo.html](crates/web/timestamp-demo.html) 데모는 이 계약을
시간 범위 변경, 차트 폭 변경, export scale 변경과 함께 가장 빨리 눈으로
확인하는 경로다.

### `AxisTitleOptions` / `ChartTitleOptions`
| 필드 | 타입 | 의미 |
|---|---|---|
| `text` | `RichText` | greek / sub/super / bold/italic 등 styled segments |
| `visible` | bool | |
| `offset_x / y` | f32 | nudge |
| `top_margin` | f32 | (chart_title only) 차트 타이틀 band 높이 |

### `GridOptions`
| 필드 | 타입 | 의미 |
|---|---|---|
| `show_major_x/y` | bool | major 그리드 라인 |
| `major_x/y_color, _width, _style` | mixed | major 라인 외형 (Solid / Dash / Dot 등 11 종 preset) |
| `show_minor_x/y` | bool | minor 그리드 라인 |
| `minor_x/y_color, _width, _style` | mixed | minor 라인 외형 |

### `DrawStyle`
| 변종 / JSON mode | 의미 |
|---|---|
| `Precise` / 생략 또는 `{ "mode": "precise" }` | 기본 정밀 렌더러. 기본 직렬화에서는 `draw_style` 키가 생략됨 |
| `Sketch` / `{ "mode": "sketch", ... }` | 차트 전체 손그림 스타일 |
| `Milkyway` / `{ "mode": "milkyway", ... }` | 차트 전체 천체사진 스타일. 파라미터 메타데이터는 `draw_style_param_specs("milkyway")` 에서 제공 |
| `Constellation` / `{ "mode": "constellation", ... }` | `ScatterLine` 전용 별자리 스타일. scatter 위치의 별과 이를 잇는 투명한 선만 렌더링하며, 별 크기는 scatter `point_size`를 따른다. 파라미터 메타데이터는 `draw_style_param_specs("constellation")` 에서 제공 |

### `Legend`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | |
| `content` | `RichText` | 범례 전체가 **하나의 리치 문서**: `'\n'` 세그먼트가 줄바꿈, 심볼은 세그먼트별 `color` 오버라이드를 가진 인라인 세그먼트 — 줄바꿈·심볼 위치·글자 중간 심볼이 전부 SSoT에 명시적. `font` / `font_size` 는 그리기 시점에 적용 |
| `corner` | `LegendCorner` | `TopLeft / TopRight / BottomLeft / BottomRight` |
| `padding` | f32 | legend box 내부 padding. corner 배치는 고정 data-area inset과 `offset_x / offset_y`를 사용 |
| `bg_color, border_color` | `Color` | 박스 배경 / 테두리 |

심볼은 **고정폭 필드 세그먼트**(`field_em`)다: 형태와 무관하게 모든 심볼이
정확히 `SYMBOL_FIELD_EM`(2.0 em × 폰트 크기)을 차지한다 — 선 마크는 필드를
가득 채우는 그려진 선(`rule: true`), scatter 마크는 필드 중앙의 shape
글리프(`● ■ ▲ …`), 선+점은 rule–글리프–rule 합계가 같은 폭. 점선/도트
선 스타일은 rule 세그먼트의 `rule_dash` 로 보존되어 범례 기호도
`LineStylePreset` 을 반영한다. 자동 구성
엔트리는 `심볼 + ' ' + '\t' + 라벨` 형태라 라벨도 탭 열로 정렬된다.
구성 헬퍼: `symbol_segments(kind, color)`, `series_symbol_segments(cfg)`,
`append_legend_entry(content, symbol, label)`.

### `PickedPointsConfig`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | `picked_points`가 있을 때 overlay 표시 여부 |
| `refs` | `Vec<PickedPointRef>` | 선택된 데이터 참조: `series_id`, 선택적 `source_id`, `point_index`. overlay는 좌표 복사본이 아니라 provenance만 저장한다 |
| `ring_color` | `Color` | 강조 링 색 |
| `ring_width_px` | f32 | 링 stroke 픽셀 두께 |
| `radius_extra_px` | f32 | 원본 마커 바깥에 더하는 추가 반지름 |

`picked_points` 누락 / JSON `null` 은 picked-point overlay 없음이다. JSON `{}` 는 기본 overlay 설정(`visible: true`, 빈 refs, 금색 링, 2 px stroke, +3 px radius)으로 파싱되므로, 호스트가 overlay를 켠 뒤 `refs`만 채울 수 있다.
overlay ring은 선택된 scatter marker 반지름을 따른다(포인트별 스타일 매핑 포함). line-only pick은 스냅된 endpoint 주변에 `radius_extra_px`만 사용한다.

### `DataSelectionsConfig`

`Config.picked_data`는 `Point`, `HistogramBin`, `MatrixCell`, `ContourLevel`
tagged `PickedDataRef` identity를 보관한다. 모든 ref는 `series_id`와 선택적
`source_id`를 가지며, kind별로 `point_index`, `bin_index`, canonical
`x_index/y_index`, 또는 `level_index + x_index/y_index`만 추가한다. 시각 정책은
`highlight_color`, `outline_width_px`, `point_radius_extra_px`,
`contour_width_extra_px`다. 좌표·막대 경계·컨투어 segment는 저장하지 않고 일반
draw와 동일 GPU 자원에서 현재 형상을 푼다. 범위를 벗어난 stale index는 그리지
않는다. JSON `null`은 해제, `{}`는 빈 금색 기본 overlay다.

### `ColorBarOptions`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | `false`면 아무것도 그리지 않고 **밴드도 반납**한다(면 시리즈는 정상 렌더) |
| `side` | `Side` | `Left`/`Right` = 수직 바, `Top`/`Bottom` = 수평 바. 이것만으로 방향이 결정된다 |
| `thickness_px` | f32 | 스트립의 짧은 쪽 |
| `gap_px` | f32 | 데이터 영역과 스트립 사이 |
| `length_frac` | f32 | 그 변 길이에 대한 스트립 길이 비율. `(0, 1]` |
| `align` | `BarAlign` | 변을 따라 `Start` / `Center` / `End` — 앵커의 이산 절반 |
| `offset_x`, `offset_y` | f32 | 그 앵커에서의 자유 이동, 화면 픽셀. **마진에 기여하지 않는다** — `Legend::offset_{x,y}` · 타이틀 · 라벨 offset과 같은 계약이라 바를 끌어도 데이터 영역이 다시 흐르지 않는다. 드래그가 누적되는 곳 |
| `colormap` | `ColorMap` | `Viridis` / `Magma` / `Turbo` / `GrayScale` / `RdBu` / `Custom { stops }` |
| `nan_color` | `Color` | 램프에 놓을 수 없는 z(NaN, 로그 컬러바의 비양수)의 색. 기본 완전투명 |
| `border_color`, `border_width` | `Color`, f32 | 스트립 테두리 |
| `axis` | `AxisOptions` | **z 축 — z 범위의 단일 진실 원본** |

`axis`가 4축과 같은 `AxisOptions`인 것이 이 설계의 핵심이다. `scale`
(`Logarithmic` 포함) · `min`/`max` · `major_spacing` · `minor_count` ·
`label_style`(`LabelFormat::Power` 포함) · `tick` · `title_option`이 축과
완전히 같은 의미이고, 틱 생성 · 라벨 포맷 · 로그 처리가 **같은 코드**를
지난다(평행 구현이 아니다).

따라오는 규칙 — 전부 결정 사항이다:

- `Heatmap` / `Contour` / `HeatmapContour` 시리즈가 있으면 이 키가 **있어야
  한다.** 없으면 z 범위도 colormap도 어디에도 없어 그릴 값 자체가 없으므로
  렌더러가 시리즈를 거부한다(추론해서 만들지 않는다).
- 결과적으로 **차트당 z 스케일 1개**다. 히트맵 여러 개는 같은 스케일을 공유한다.
- 밴드 = `gap_px + thickness_px + axis.out_margin + axis.major_tick_length`,
  그 변에만 더해진다. `fit_to_data_area` / `resize_chart_area_scaled`는
  `axis.out_margin`(축과 같은 라벨 공간)만 조절하고 스트립 자체는 건드리지
  않으므로 컬러바가 창 크기에 따라 얇아지지 않는다.
- 4축과 기본값이 다른 두 곳: `line_visible: false`(스트립 테두리가 그 선 역할)와
  `tick: Outside`(틱이 색 위가 아니라 라벨 마진에 놓인다).
- 밴드는 그 변 마진의 **맨 바깥**이다. 차트 가장자리에서 안쪽으로 컬러바 라벨 마진 → 컬러바 틱 →
  스트립 → `gap_px` → 그 변 축의 밴드 순이다. 축의 틱 라벨은 데이터 영역에서 바깥으로 그려져
  비켜줄 수 없으므로, 스트립을 데이터 영역 옆에 두면 그 라벨 위에 그려진다.
- 컬러바는 GPU 파이프라인 없이 decoration 레이어에서 CPU로 그린다. 틱·라벨·타이틀이 4축과 같은
  헬퍼를 지나므로 로그 컬러바는 decade 틱과 10ⁿ 라벨을 로그축이 이미 쓰는 코드에서 얻는다.
  `axis.tick`이 안/밖/양쪽을, `axis.inverted`가 화면의 min→max 방향을 정하고, 틱 외형은 축선과
  같은 `line_color` / `line_width` / `line_style`을 그대로 쓴다.
- 다른 크롬과 같은 **선택 · 드래그 · 리사이즈** 요소다: 히트테스트 id `"colorbar"`,
  파란 선택 박스, 그리고 데이터 영역과 함께 **8개 리사이즈 핸들을 가진 둘뿐인 요소**다.
  드래그는 `offset_{x,y}`에 누적되고, 핸들은 **바의 방향**에 따라 `thickness_px` 또는
  `length_frac`을 움직인다(핸들은 화면 방향만 알고, 어느 치수인지는 nudge가 푼다). 세부 요소는
  `"colorbar_axis"`, `"colorbar_tick_labels"`, `"colorbar_title"`로 각각 선택·표시되며,
  드래그는 차례로 `axis.line_offset`, 라벨 offset, 제목 offset을 바꾼다. 셋 모두 실제 스트립
  사각형에서 파생되므로 길이·정렬·이동·리사이즈 뒤에도 제목과 히트박스가 바를 그대로 따른다.
- `ColorBarOptions::normalized_z(z) -> Option<f32>`가 z→색 정규화의 단일 원본이고
  `color_for_z`가 그것을 적용한다. `[0,1]` 클램프이며, NaN · 로그 바의 비양수 · 퇴화 범위는
  `None` → `nan_color`로 그린다(끝점으로 클램프하지 않는다 — "없음"과 "가장 작음"은 다른
  사실이다). `axis.inverted`는 적용하지 않는다: 값을 어디에 그리는지를 바꾸고 어떤 색인지를
  바꾸지 않는다.

### `data_config` — series 선언형 스키마 (활성 API)

차트별 시리즈는 모두 `data_config::SeriesConfig` 로 선언. `Renderer::paint` 가 `render_type` enum 변종으로 분기해 line / scatter / errorbar layer 를 자동 생성, 색·두께·shape 등 모든 시각 속성도 sub-style 에서 추출.

| 타입 | 필드 | 역할 |
|---|---|---|
| `SeriesConfig` | `series_id, source_id?, label, x_column: ColumnId, y_column: ColumnId, render_type` | 한 시리즈의 모든 선언. `source_id`는 picking용 선택적 host provenance이고, `x_column / y_column`은 렌더러에 등록된 id로 상주 pool이나 재공급 가능한 비상주 소스를 가리킨다. web 편집 플로우에서는 `legend.content`가 live 라벨 권위이며, 일반 시리즈 편집은 인식 가능한 범례 심볼만 갱신하고 사용자 텍스트를 보존한다. `SeriesConfig.label`은 명시적 `reset_legend_from_series_labels()` 재작성에서만 권위가 된다 |
| `DataRenderType` | 13 변종 enum | 변종별 독립 draw path. 옵셔널 struct 안 합침 |
| `ErrorRef` | `Symmetric { column }` 또는 `Asymmetric { lower, upper }` | 에러바 컬럼 참조. Symmetric 은 ±σ, Asymmetric 은 lower/upper 분리 |
| `DataLineStyleConfig` | `line_style, line_color, line_width` | 라인 외형 |
| `DataScatterStyleConfig` | `point_color, point_shape, point_size, point_style_table?, point_style_index_column?, point_style_overrides?` | 점 외형. optional style map은 precise scatter에만 적용되며 table/override slot이 색, shape, 크기 또는 일부만 대체할 수 있다 |
| `DataErrorBarStyleConfig` | `error_bar_color, _width, _cap_size, cap_width, error_bar_style_table?, error_bar_style_index_column?, error_bar_style_overrides?` | 에러바 외형. optional style map은 precise errorbar에만 적용되며 table/override slot이 색, stem width, cap half-size, cap width 또는 일부만 대체할 수 있다 |
| `DataBarStyleConfig` | `fill_color, border_color, border_width, baseline, gap_px, width_ratio, orientation, bar_style_overrides?` | 히스토그램 외형. `width_ratio`는 bin 안에서 가운데 정렬된 막대 비율이고 sparse override는 특정 bin의 채움·외곽선·간격·폭을 바꾼다 |
| `ScatterShape` | enum 26 변종 | Circle / Square / Triangle directions / Diamond / Cross / Plus / Pentagon / Hexagon / Octagon / Star + filled variants |

**`DataRenderType` 변종 13 개**:

| 변종 | 사용 sub-style | 의미 |
|---|---|---|
| `Line { line }` | line | 라인만 |
| `Scatter { scatter }` | scatter | 점만 |
| `ScatterLine { scatter, line }` | 둘 다 | 점 + 연결선 |
| `ScatterErrorbarX { scatter, err_x, err_style }` | scatter + errorbar | 점 + X 에러바 |
| `ScatterErrorbarY { scatter, err_y, err_style }` | scatter + errorbar | 점 + Y 에러바 |
| `ScatterErrorbarXY { scatter, err_x, err_y, err_style }` | scatter + errorbar | 점 + X/Y 에러바 |
| `LineScatterErrorbarX / Y / XY` | line + scatter + errorbar | 위 3 + 연결선 |
| `Histogram { bar }` | bar | 호스트가 비닝한 `(edges, counts)` 막대. `bar.orientation`이 컬럼 역할을 **단독 결정**한다: `Vertical` = `x_column` edges / `y_column` counts, `Horizontal`은 반대. 길이 관계(`edges = counts + 1`)로 추측하지 않는다 |
| `Heatmap { matrix, fill }` | fill | 면만 |
| `Contour { matrix, contour }` | contour | 선만 |
| `HeatmapContour { matrix, fill, contour }` | fill + contour | 면 + 그 위의 선 |

히스토그램 폭은 먼저 `width_ratio`로 bin의 가운데 정렬된 `0..=1` 비율을
남기고, 그다음 `gap_px`를 화면 픽셀 단위로 추가 차감한다. 단, 양수 폭 막대가
1픽셀보다 넓으면 최소 1픽셀을 남기도록 gap을 제한한다. 원래 bin 폭이 화면에서
1픽셀 미만이면 GPU가 각 픽셀 열에 겹치는 bin의 최댓값을 선택해 0까지 채운다.
가로 히스토그램은 픽셀 행 기준이며, 채움은 표시 중인 축 범위에서 잘린다.
최댓값 bin의 선 두께와 알파가 양수면 영역 전체를 선 색으로 채우고,
그렇지 않으면 면 색으로 채운다. 동률이면 앞선 bin을 선택한다.
이 경로에서는 gap과 양수 width_ratio로 틈을 만들지 않는다. 원본 컬럼은 유지하며
`width_ratio = 0`인 bin은 제외한다. `border_width = 0`이면
외곽선이 꺼지고, 양수이면 `border_color`와 두께가 적용된다.
`bar_style_overrides`는 `index`로 특정 bin을 고르는 sparse 목록이며 각 항목이
`fill_color`, `border_color`, `border_width`, `gap_px`, `width_ratio` 중 필요한
값만 덮는다. baseline·orientation은 시리즈 공통이고, 렌더·typed pick·선택
outline은 모두 같은 최종 막대 경계를 쓴다.

매트릭스 3종은 격자를 `MatrixRef { columns, orientation, grid_layout }` —
등록된 컬럼 id 묶음 — 으로 선언한다. 별도의 매트릭스 컨테이너는 없다.
상주 경로는 pool의 컬럼을 읽고, 지원되는 비상주 Heatmap 경로는 같은 ID의
원본 구간을 제한된 크기로 재공급받는다. 어느 경로도 `Config`나 `series`를
복제하지 않는다. `grid_layout`은
좌표 컬럼이 셀 경계(`Edges`, n+1)인지 중심(`Centers`, n)인지를 말하며 길이로
추론하지 않는다. 선언과 데이터의 개수가 어긋나도 **에러가 아니다** — 가장 작은
공통 범위까지 그리고 잘렸다는 사실만 알린다.

<!-- contour-contract: scope=readme-ko max-levels=1024 -->
`ContourConfig.levels`는 항상 데이터 단위의 명시 목록이다(자동 추론 variant
없음 — 그리는 시점에 추론한 레벨은 config에 없는 값이다). `per_level_color:
None`이면 모든 레벨이 `line.line_color` 단색이고, colormap에서 레벨 색을
유도하지 않는다. 허용 길이는 `0..=1024`이며 1025개 이상은 오류이고 어떤
레벨도 조용히 잘라내지 않는다. 캐시가 빗나갈 때 원본 목록과 선언 순서는
유지한 채 연속된 32개 블록별 값 정렬 검색 복사본을 만든다. fragment는 최대
32개 블록을 이진 탐색하고 실제 도달 가능한 후보만 계산한다. 한 셀에 1024개가
실제로 모두 걸리면 선언 순서대로 1024개 전부를 합성한다. 선 자체는 면의
이중선형 보간의 level set으로 그린다. 거리는 현재 셀의 이중선형 field를 현재
gradient-normal 직선으로 제한해서 얻는 이차방정식 교차근이며, 전체
piecewise-bilinear contour에 대한 전역 최단거리는 아니다.

비정상 레벨의 의미는 실제 업로드된 f32를 기준으로 명시된다. contour line은
NaN과 양쪽 Infinity를 모두 제외한다. `FillMode::Bands`의 분자는 `-Infinity`
전체와 z 이하인 유한 레벨만 세고, 분모는 선언된 전체 레벨 수를 유지한다:
`t=(negative_infinity_count + finite_le_z + 0.5)/(declared_level_count + 1)`.
따라서 NaN과 `+Infinity`는 분모에만 영향을 준다.

`ContourLabelConfig.anchors`는 **오버라이드**다. 비어 있는 것이 정상이고, 그때는 GPU가
데이터 영역에 `spacing_px` 격자로 씨앗을 놓고 각 씨앗을 자기 레벨의 isoline으로 투영한 뒤
정상 선택에서는 `spacing_px` 간격을 목표로 남긴다. 단, 레벨별 fallback은 레벨을
누락시키지 않기 위해 더 가까운 후보를 남길 수 있다. `spacing_px`는 숨김 라벨과 명시
오버라이드에서도 항상 유한한 양수여야 한다. 자동/명시 배치는 공통으로 1024개 용량을
쓴다. 명시 앵커는 유효하지 않은 `level_index`를 버리고 입력 순서에서 유효한 앞
1024개만 남긴다. 이 resolved 목록이 비면 자동 배치하고, 하나라도 남으면 그 목록이
자동 배치를 대체한다. 배율은 자동 배치 spacing에만 곱하며 export scale을 clamp한 뒤
그 곱이 유한한 양수인지 검사한다. atlas도 adapter의 texture dimension 한계 안에
들어야 한다. 비정상 spacing, scale 곱 overflow, atlas 초과는 renderer 상태를 발행하기
전에 실패하므로 이전 chart와 GPU 자원이 유지된다. 앵커는 데이터 좌표 + 데이터 공간
접선이라 줌/팬 때 다시 투영만 하면 된다.
`ContourLabelConfig.color`가 선과 `per_level_color`에서 독립적으로 글자색을 소유한다.
Decimal 표기는 Bottom X가 아니라 contour level 간격(없으면 colorbar 간격)을 쓰며,
`significant_digits`도 실제로 적용하되 인접 레벨이 같은 문자열로 뭉개지지 않게 한다.
contour fragment는 라벨 draw와 같은 선택 앵커 버퍼를 읽어 라벨 사각형 안의 선을 실제로
그리지 않는다. `bg_padding_px`는 `bg_color`가 없어도 이 선 간격을 패딩한다.

`Renderer::series_draw_info(chart, series_id) -> SeriesDrawInfo`가 **실제로 그려진 것**을 보는
시리즈 공통 창구다: `drawn_count` · 매트릭스의 `cols`/`rows` · `truncated`. 컬럼 길이는 SSoT가
아니라 데이터에서 오는 사실이므로 불일치는 에러도 중단도 아니다 — 가장 작은 공통 범위까지 그리고
여기서 알린다. 호스트가 "edges 11개 / counts 9개였으니 막대 9개를 그렸다"를 알게 되는 경로이고,
line·scatter의 기존 `min(x, y)` 잘림도 같은 호출로 설명된다.

이 4종은 point-only 호환 picker를 지나지 않는다. 히스토그램은 업로드된 edge/value
메타데이터로 fit한다. 매트릭스는 별도 field mode로 같은 GPU fit 엔진을 쓰며, CPU는
잘림까지 반영한 셀 개수만 넘기고 GPU가 field shader와 같은 좌표 pair 풀에서
`Edges`/`Centers` 및 cell/sample lattice 규칙을 그대로 적용한다. 그래서 contour와
interpolated fill은 실제 sample 끝(`Edges`에서는 경계 좌표의 중점)에, flat fill은 실제
셀 경계에 정확히 맞는다. 선택은 `pick_data`의 bar/field shader entry가 맡는다.

**`Renderer::create_style_for_series(cfg)`** 가 `cfg.render_type` 의 sub-style 에서 색/두께/shape 자동 추출 → GPU `ChartStyle` 빌드. 화면 paint 시 사용. export 는 `create_style_for_series_scaled(cfg, scale)` 로 두께만 픽셀 스케일.

**한쪽 차원만 errorbar 시** (`ScatterErrorbarY` 등): 방향 유무는 `PrimitiveStyle::primitive_flags`의 Y=bit 0, X=bit 1로 명시한다. 미사용 vertex slot은 이미 바인딩된 anchor 컬럼을 재사용하며 셰이더가 error attribute를 읽기 전에 해당 방향을 접는다. 따라서 prepare/export가 숨은 filler 컬럼이나 host metadata를 만들지 않는다. 실제 오차값 0은 “방향 없음”이 아니라 길이 0인 유효 errorbar다. (Symmetric 변종은 같은 error 컬럼을 lo/hi 양쪽에 사용.)

### `Config::scaled(scale)` / `Config::scale_in_place(s)`
모든 픽셀 dim 을 `scale` 배. `min/max/major_spacing`, scale enum, 색은 무변경. 고해상도 export 시 시각적 동치 보장.

### 기본값 빌더 — `renderer::default::default_config()`
- bottom_x / left_y: 축선 + 눈금 + 라벨 + 타이틀 활성, 텍스트는 빈 segments.
- top_x / right_y: 축선 + tick 활성, 라벨 + 타이틀 비활성, `out_margin = 8` (좁은 gap).
- chart_title: visible, top_margin 32, 텍스트 빈 segments.
- grid: major 만 활성, 옅은 회색.
- legend: 비활성.

빈 텍스트는 `Chart::with_title / with_x_title / with_y_title / with_legend_entry` 빌더로 채움.

---

## 3. 내부 메모리 데이터 흐름

![figgy 내부 메모리 및 렌더링 아키텍처](crates/renderer/assets/architecture-state-flow-kr.png)

### 원본 데이터 스트리밍과 상주 전환

![figgy 스트리밍 기본 경로: 원본 소스, 웹 실행기, 렌더러 상태, 청크 누적 화면](crates/renderer/assets/streaming-architecture-en.png)

그림은 **기본 화면 표시 경로**를 나타내며 선택적인 차트별 패킹 뷰 캐시는 아직
표시하지 않는다. 원본 데이터는 호스트가 재공급 가능한
TypedArray 또는 `readRange` 공급자로 보관한다. 웹 facade는 브라우저 실행
일정과 구간 요청을 연결하지만 스트림 커서·차트 상태·통계의 권위는 갖지 않는다.
렌더러는 `Config`, 순서 있는 시리즈, 소스 revision, 커서, 범위 요약 캐시와
현재 뷰의 상주 가능 여부를 소유한다. 자동 스트리밍에서는 연결된 컬럼 전체를
`ColumnPool`에 승격하지 않는다. 제한된 청크를 GPU에 올려 오프스크린 면에
누적하고, 설정된 예산에 맞으면 현재 화면에 필요한 원본 행만 GPU에 유지한다.
두 경로 모두 원본 primitive를 그리며 LOD·샘플링·데시메이션은
하지 않는다. 같은 페이지에 상주 차트와 스트림 차트를 함께 둘 수 있다.

스트림은 완료된 부분부터 화면에 보여 준다. 변경 없는 완료 revision은 결과를
재사용하고, 제목·축 이름 같은 장식 변경은 데이터 커서와 누적면을 유지한다.
더 좁은 뷰는 패킹 캐시에서 원본을 다시 읽지 않고 그릴 수 있다. 캐시 범위를
벗어난 뷰나 물리 해상도 변경에는 동일 revision의 원본을 다시 공급한다.
`job.cancel()`은 새 작업을 중단하고 제출된 GPU 작업이 끝난 뒤 해당 자원을
정리한다. 완료된 패킹 뷰의 점·선 피킹은 패킹된 GPU 행을 조회해 원본 행 인덱스를
반환한다. 그 밖의 비상주 스트림에는 즉시 피킹을 제공하지 않는다. 배율을 지정한
PNG 출력은 화면 누적 이미지를 늘려 쓰지 않고 원본 구간을 다시 공급받아 그리므로,
호스트는 재생과 출력을 위해 원본을 유지해야 한다. 지원 범위와 웹 API는
[WASM 가이드](crates/renderer/WASM.md#exact-streaming)에
정리했다. 원본 데이터를 빠짐없이 처리한다는 뜻과 GPU 백엔드·렌더 패스 경계의
안티앨리어싱 픽셀이 바이트 단위로 같다는 뜻은 구별해야 한다.

renderer-owned registry가 browser wrapper의 지속 SSoT다. 저수준 native host는
`ChartDrawItem`을 직접 전달할 수도 있다. 어느 경로든 `ChartDrawItem`은
prepare 전용 입력이고 paint는 owned token만 소비한다.

### 소유권과 수명 경계

`Renderer`는 chart registry와 GPU 측 상태의 수명 소유자다. chart별 권위
`Config`와 순서 있는 `SeriesConfig`, 상주 `ColumnPool`, 0.12.0 후보의 비상주
논리 소스 metadata, render/compute pipeline,
공유 picker pipeline bundle, 최대 하나의 파생 active-chart picker cache, pending
pool maintenance, bind group, panel별 `ChartView` / `ChartStyle` GPU 자원, 공유
`Arc<wgpu::Device>` / `Arc<wgpu::Queue>`를 보관한다. `ChartId`는 발급한
renderer에 결박된 opaque id다. 하나의 논리 편집이 Config와 ordered series를
함께 바꾸면 `set_chart_state`가 두 값을 검증하고 원자적으로 교체한다.

column upsert, 제거, defrag는 실패 가능한 pool/chart/revision/active-picker
준비를 모두 끝낸 뒤 새 권위 상태를 공개한다. 동기적으로 반환되는 오류에
대해서는 기존 pool/chart/picker가 그대로 유지되고, 마지막 공개 단계에는
allocation이 없다. plain `remove_column`은 그 id를 참조하는 모든
renderer-owned series를 cascade 제거하지만 어떤 `Config::legend` 문서도
고치지 않는다. cascade 결과에 따른 범례 변경도 함께 적용해야 하는 host는
`remove_column_with_chart_config`를 사용한다. 이 API는 pool, 영향받는 모든
series, 해당 chart의 교체 `Config`를 같은 transaction에서 공개한다. web
facade는 auto-managed/free-edited legend 정책에 이 결합 경계를 사용한다.

Renderer 0.9의 exact GPU picking은 chart-aware API다.
`enable_gpu_picking()`을 한 번 호출하고, 필요하면
`prepare_gpu_picking_for_chart(chart_id)`로 첫 pick 전에 chart registry를
준비한 뒤 `pick_chart(chart_id, GpuPickRequest)` 또는
`WindowedRenderer::pick_chart_at`으로 제출한다. 축 transform과 data-area clip은
renderer가 권위 `Config`에서 계산한다. 0.7에서 공개했던 저수준
`GpuPickEngine` 표면은 더 이상 노출하지 않는다. picker는 GPU column pool을
직접 읽으며 CPU point mirror와 내부 `Mutex`를 두지 않는다.
tagged point/bin/cell/contour 결과는 `pick_chart_data` /
`WindowedRenderer::pick_chart_data_at`을 사용한다.

모든 변경 — `Renderer::prepare` 와 export prepare 경로 — 은 `&mut self`
경계에서 실행되고, renderer 내부에 새 공유 락을 만들지 않는다.
`Renderer::paint_prepared` 는 owned `PreparedFrame` 토큰을 상대로 `&self`
로 기록하므로, 공유 참조만 제공하는 host paint 콜백에도 wrapper 락이
필요 없다 (`Renderer` 는 `Send + Sync`).

토큰은 resolve된 pipeline, bind group, buffer, panel geometry, column
allocation epoch, pool layout generation, target-pipeline generation,
캡처한 각 `ChartView`의 content revision을 보유한다. 하나라도 어긋나면
기록 전에 `FiggyError::StalePreparedFrame`으로 실패하고 다음 frame에 다시
prepare한다. arc/star scratch와 automatic contour placement는 compute 입력
bit key별 불변 결과다. 같은 입력만 공유하고 geometry, data generation,
placement 입력이 달라지면 새 결과를 만들며 기존 결과는 다시 쓰지 않는다.
cache나 prepared token이 대응 GPU handle을 소유하는 동안 shared charge도 함께
살아 있고, 마지막 Figgy owner가 drop된 뒤에도 retired 회계는 host가
`end_gpu_frame()`으로 queue submit을 알릴 때까지 유지된다. explicit placement도
불변이다. 같은 `ChartView` 재작성, 캡처 column 교체,
pool defrag, target pipeline 재생성은 기존 토큰을 의도적으로 stale 처리한다.
host는 그 변경 전에 기존 토큰으로 기록한 command buffer를 제출해야 한다.

상주 `add_column`의 `ColumnSource` 데이터는 upload 순간에만 빌려 읽힌다.
장기 보관되는 것은 GPU pool column과 auto-fit 용 scalar stats(min / max /
최소 양수)뿐이며, 원본 source 참조나 CPU 측 per-point geometry는 유지하지
않는다. dashed line 또는 constellation arc prefix 같은 per-point geometry는
GPU pool을 compute scan해서 만든다. 비상주 등록은 논리 소스 metadata와
제한된 GPU 작업 상태를 유지하고, 호스트가 정확한 재생을 위해 원본 구간
공급자를 보관한다. 렌더러는 원본 전체의 CPU 사본을 보관하지 않는다.

진행 중인 `GpuPickTicket`은 readback 자원과 제출 시점의 `Arc` 기반 identity
mapping을 직접 소유한다. 이후 chart/pool이 변경되거나 renderer가 drop되어도
그 ticket이 반환할 `source_id` / `series_id`가 다른 대상으로 바뀌지 않는다.

web public surface도 같은 경계를 따른다. `<figgy-chart>` facade가 shadow
canvas, ready promise, rAF loop, ResizeObserver/DPR 처리, pointer mapping,
async operation busy gate, id 등록/해제 수명주기를 소유한다. raw `FiggyChart` wasm
kernel은 advanced escape hatch로 남는다.

웹 cold-start와 lifecycle 계약:

| 표면 | 계약 |
|---|---|
| raw `FiggyChart` | wasm 브라우저의 `create` / `create_with_progress`는 동일 `GPUDevice`의 모든 render WGSL entry를 Promise 기반 `createRenderPipelineAsync`로 데우고 임시 JS pipeline을 버린 뒤 빈 차트 첫 frame을 submit하고 완료까지 기다린다. Production renderer-owned optional render/style과 arc/fit/picker/contour compute cache는 lazy 상태를 유지한다. `prewarm_all_with_progress(callback)`은 `{ scope, stage, phase }` progress와 함께 실제 wgpu cache를 게시하고, `prewarm_all()`은 callback 없이 같은 작업을 한다. `warm_up()`은 first-frame compatibility alias이며 full prewarm이 아니다. create는 production picker를 enable하지 않는다. `prewarm_gpu_picking()`이 명시적으로 enable하고 현재 chart를 준비하며, 재시도와 `pick_point` / `pick_data`는 sticky activation error를 포함한 같은 renderer-owned 경로를 재사용한다. |
| `<figgy-chart>` 시작 | `web.create / first frame / finished` progress event와 `figgy-ready`를 공개한 뒤 background picker prewarm을 시작한다. 실패하면 `operation: "prewarm_gpu_picking"`, `recoverable: true`인 `figgy-error`를 내보내지만 이미 fulfilled된 `ready`와 rendering loop는 유지한다. |
| async 직렬화 | generation+kernel operation token 하나가 connect/create, `prewarm_all_with_progress` / `prewarm_all`과 picker prewarm, export, `first_frame_ready` / `warm_up`, extent 준비, async fit, pick을 포괄한다. facade의 두 full-prewarm 메서드도 기존 generation-aware operation gate를 통과한다. `busy` 동안 rAF draw와 pointer/proxy kernel 접근은 wasm에 들어가지 않고, 최신 resize 하나와 pending pointer release만 보관해 settle 뒤 적용한다. |
| disconnect/reconnect | disconnect는 generation을 무효화하고 해당 rAF/observer를 해제한다. active operation이 빌린 kernel은 operation settle 뒤에만 free한다. stale settle은 새 generation의 kernel token을 해제하거나 resize/release/free하지 못한다. |

웹 mutation API 계약:

| API | 계약 |
|---|---|
| `auto_fit_colorbar(padding)` | 모든 matrix value column의 upload metadata 합집합에 공유 colorbar z축을 맞춘다. colorbar가 없는 chart는 변경하지 않는다. |
| `set_colorbar_axis(json)` | 기존 컬러바의 `AxisOptions` SSoT 전체를 교체한다. 틱 외형/안팎 방향/길이, 축 반전, 틱 라벨 스타일·offset, 제목 옵션을 한 경계에서 편집한다. |
| `set_colorbar_title(text)` | 컬러바 제목을 설정하고 보이게 한다. 빈 문자열은 숨긴다. `Config.colorbar`가 없으면 실패한다. |
| `set_contour_nice_levels(series_id, target_count, use_colormap_colors)` | colorbar 축의 tick 규칙으로 한 contour series의 explicit level을 교체하고, 선택적으로 color-map 색을 지정해 series SSoT에 기록한 뒤 level 수를 반환한다. |
| `series_draw_info(series_id)` | `{ drawn_count, cols, rows, truncated }`를 보고한다. raw wasm `FiggyChart`는 JSON 문자열을 반환하고 `<figgy-chart>` facade는 이를 파싱한 object를 반환한다. |
| `pick_data(x, y, max_distance_px)` | point/bin/cell/contour tagged identity 또는 `null`을 비동기로 반환한다. raw wasm은 JSON string/`undefined`, facade는 object/`null`이다. |
| `set_picked_points(json)` | `PickedPointsConfig` 또는 `null`을 인코딩한 JSON 문자열을 받는다. renderer-owned `Config.picked_points`만 교체하며 `null`은 overlay를 지운다. 참조는 복제한 좌표가 아니라 `series_id`, 선택적 `source_id`, `point_index`를 보관한다. |
| `set_picked_data(json)` | `DataSelectionsConfig` 또는 `null`을 받아 `Config.picked_data`만 교체한다. ref는 provenance와 stable index만 보관하고 현재 geometry는 GPU-backed chart SSoT에 남는다. |
| `set_clear_color(r, g, b, a)` | linear RGBA 각 성분을 받아 `0..1`로 clamp하고 surface redraw를 예약한다. clear color는 host/surface 상태이므로 Config JSON을 바꾸거나 axis raster refresh를 강제하지 않는다. |

public `FiggyChart::load_demo()`는 compound failure-atomic 호출이다. 4개 컬럼,
최종 `Config`/ordered series, active picker, host metadata, 유효한 extent cache가
한 번에 보이며, 동기 준비 오류가 나면 이전 전체 상태가 그대로 남는다. 이
transaction 안에서는 extent reduction을 submit하지 않고, 무효화된 extent는
commit 뒤 기존 lazy/retry 경로가 다시 만든다. 승인된 호출은 pool capacity와
같은 임시 GPU buffer 하나와 staging buffer 4개를 추가로 사용한다. 기존 defrag
backup이 있으면 순간 pool 저장량은 primary + backup + 임시 full-pool buffer다.

이 소유권 규칙은 데이터 무왜곡 계약과 연결된다. renderer/web은 source
column을 변형 저장하지 않고, clipping, log-domain skip, NaN skip,
antialiasing 한계는 데이터 재작성이 아닌 렌더링 결정으로만 적용된다.

### 점선 호장 스캔 (GPU)

dash 위상은 매 점의 누적 픽셀 호장이 필요하고, 이는 라이브 데이터→픽셀
변환에 의존한다. 서로 다른 compute key마다 GPU에서 생산하고, 정확히 같은
key는 불변 결과를 재사용한다:

```
pool 컬럼 (x, y) ──┐                        Transform uniform (96 B write)
                   ▼                                   │
   seg_init        dst[i] = |px(pᵢ) − px(pᵢ₋₁)|   ◄────┘
   scan_block      256-블록 inclusive 스캔 (Hillis–Steele, 공유 메모리)
   scan_block/add  블록 합 레벨 (dst → sums0 → sums1)
   carry 체인      min(디스패치 한계 × 256, 256³) 점 단위 청크를 순차 실행;
                   1-원소 carry 버퍼가 각 청크의 누계를 다음 청크에 전파 —
                   n 의 상한은 풀 메모리뿐, 어떤 크기에서도 readback 없음
                   ▼
   호장 prefix buffer ──► 라인 파이프라인 정점 슬롯 4/5 (dash 위상)
```

컴퓨트 인코더는 호스트의 렌더 패스보다 먼저 submit 되므로 큐 순서가 모든
임베딩(winit / egui / iced / web)에서 API 변경 없이 순서를 보장한다.
정확한 key는 pool layout generation, x/y offset과 allocation epoch, length,
compute shader가 읽는 geometry transform bit 전체, optional star pitch를 포함한다.
시리즈마다 최근 불변 결과 8개를 보관하며 miss는 새 buffer에 dispatch하고 옛
결과를 절대 덮어쓰지 않는다. 현재 arc-prefix
scan은 u32-addressable 범위(`u32::MAX = 4,294,967,295`) 안에서 동작한다.
시리즈 길이나 pool element offset이 `u32`에 들어가지 않으면 dashed arc
prefix는 생략된다. 새 series id를 삽입할 때 arc cache가 이미 256개 series id를
보유하면 runaway churn 방지를 위해 전체 arc cache를 clear한다.

### Renderer-owned 상태와 frame invalidation

지속 host는 chart를 `Renderer`에 등록하고 실제 submit/present에 성공한 마지막
`ChartRenderStamp`를 보관한다. checked revision은 renderer만 발급한다:

| 상태 | 현재 trigger / 처리 |
|---|---|
| renderer `desired` revision | 승인된 시각 상태 변경, 참조 column 교체, 동기화된 font 등록은 draw 요구 |
| renderer `raster` revision | Config, series, selection, font 변경은 보수적으로 draw 전 `refresh_axis` 요구 |
| host `view_dirty` | surface/DPR preview geometry 변경 — raster refresh + draw |
| host `redraw_pending` | clear color 같은 host-only surface 상태 변경 — chart 상태 복제 없이 draw |

browser frame 흐름:

```rust
renderer.sync_external_invalidations()?;
let stamp = renderer.chart_render_stamp(chart_id)?;
let draw = stamp.needs_draw_since(last_presented_stamp.as_ref());
let raster = stamp.needs_raster_since(last_presented_stamp.as_ref());

if !draw && !view_dirty && !redraw_pending {
    process_maintenance_without_surface_if_needed()?;
    return Ok(());
}
if raster || view_dirty {
    renderer.refresh_axis_with_selection(&mut view, &display_chart, rect, &boxes)?;
}
renderer.draw(clear, &items)?;
last_presented_stamp = Some(renderer.chart_render_stamp(chart_id)?);
view_dirty = false;
redraw_pending = false;
```

last-presented stamp와 host flag는 draw 성공 뒤에만 전진하므로 visual frame
실패는 재시도된다. clean web rAF는 GPU surface 경로 전체를 생략한다. 상주
경로는 다시 그릴 때 pool의 원본 primitive를 재기록하며 data-layer image
cache를 두지 않는다. 0.12.0 후보의 비상주 경로는 부분 표시용으로 제한된 GPU
누적면을 유지하고, 바뀌지 않은 완료 revision을 재사용한다. 어느 경로도
데이터 LOD·샘플링·데시메이션을 적용하지 않는다.

독립 `Chart::{data_dirty,raster_dirty}` bool은 외부 `Chart`를 소유하는 저수준
호출자용 호환 장치로 남는다. `prepare`는 이 bool을 읽거나 consume하지 않고,
호출자가 draw를 결정했을 때 transform을 쓴다. 외부-`Chart` host는
`raster_dirty`를 consume하고 `refresh_axis`를 호출할 책임이 있다.

### Log scale GPU 처리

`AxisOptions.scale = Logarithmic` 시:
- auto-fit은 데이터에 0/음수가 섞여도 캐시된 최소 양수를 log 하한으로 사용.
- 수동 range의 0 이하/비정상 bound는 렌더러/축 경로에서 `1e-12`로 guard한다. 단, `1e-12`보다 작은 유효 양수 bound는 그대로 보존.
- CPU: `scatter_transform_from_config` 가 guard된 range를 log10 으로 미리 변환하고 해당 축의 `scale_log` 플래그를 설정.
- GPU shader: `mix(v, log10(v), is_log)` — 분기 없이 ALU로 처리. 0 이하 데이터 샘플은 data path에서 NaN/skip 처리되며 config validation 실패가 아니다.

### Export 파이프라인

```
export_panel_rgba_async(chart, &[SeriesConfig], scale).await:
    scale ← clamp_export_scale(scale)         // [MIN_EXPORT_SCALE, MAX_EXPORT_SCALE]
    chart.config().scaled(scale)               // 픽셀 dim 모두 비례 확대
        ↓
    임시 ChartView (스케일된 axis 텍스처)
    임시 ChartStyle 들 ← create_style_for_series_scaled(cfg, scale) per cfg
        ↓
    offscreen wgpu::Texture (고정 Rgba8Unorm, COPY_SRC, transparent clear)
    paint(items) — 동일 합성 순서 (grid → data → decoration)
        ↓
    copy_texture_to_buffer 를 **행 청크** 로 (256 byte 정렬 padding; 청크
    높이가 디바이스 max buffer size 에 맞춰 적응 — 초대형 export 도 동작)
        ↓
    map_async (native 는 inline Wait poll, wasm 은 브라우저 yield await)
        ↓
    premul→straight α 변환, 패딩 행 제거 (채널 스왑 없음 — 타겟이 이미 RGBA)
        ↓
    RasterImage { width, height, rgba: Vec<u8> }   ← API 반환
        ↓
    encode_png(&img) → Vec<u8>                      ← PNG 바이트
        ↓
    호출자가 std::fs::write / clipboard / 네트워크 등 자유 처리
```

---

## 라이선스 / 폰트

번들 폰트: Liberation Sans (SIL OFL 1.1) — `crates/renderer/fonts/LICENSE-LiberationSans.txt`. 추가 폰트는 런타임 등록 (wasm `register_font`, native `text_render::register_font_bytes`). byte-for-byte 동일 파일 재등록은 멱등이며, registry는 파일을 한 번만 저장하고 face id별 resolved backing을 재사용하며 global font generation을 올리지 않는다.
