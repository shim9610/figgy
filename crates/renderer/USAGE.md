# Native Rust usage / Rust 사용법

[Start here](https://github.com/shim9610/figgy/blob/master/README.md#quick-start) · [한국어 시작하기](https://github.com/shim9610/figgy/blob/master/README.ko.md#빠른-시작)

## 1. Usage

### Toolchain and builds

The repository pins **Rust 1.99.0** in
[`rust-toolchain.toml`](https://github.com/shim9610/figgy/blob/master/rust-toolchain.toml), including the wasm32 target,
rustfmt and clippy. With rustup installed, commands in this checkout select
that toolchain automatically. The package manifests declare Rust 1.99 as the
support floor; earlier toolchains have not been validated. Use the committed
`Cargo.lock` with `--locked` when building this checkout; downstream crates
resolve dependencies with their own lockfile.

```bash
cargo check --locked --workspace --all-targets --all-features
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
```

### Package names and compatibility

The Cargo packages are `figgy-renderer` and `figgy-model`, and their Rust
library names are `renderer` and `model`: code imports `renderer::...` and
`model::...`, and each dependency names its package with the `package` key
shown below.

Use the renderer's optional `serde` feature for serialization of its re-exported
model types; no direct model dependency is needed for that. Direct host GPU
sharing requires wgpu 30. The WASM wrapper (`figgy`, the browser product) is a
separate package with `publish = false`; its publication target is
[open](https://github.com/shim9610/figgy/issues/57).
See [Rust API contracts](API.md) for ownership, frame lifetimes,
render targets and compatibility policy.

### Adding the dependency

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.18.1" }
wgpu     = "30"
```

This installs `figgy-renderer` 0.18.1 from crates.io. A crate that uses the
model types without the renderer depends on
`model = { package = "figgy-model", version = "0.12.1" }`.

For a local checkout, replace `version` with a `path` relative to the consuming
application's `Cargo.toml`. A standalone program that calls `pollster::block_on`
(as [examples/line.rs](examples/line.rs) does) must also declare `pollster = "1.0"`.
Crate examples already have access to this dependency through the renderer manifest.

### Host crates

The default library features require neither winit, egui nor iced. Add only the
host crates you use (the crate's own egui examples run with `--features egui_demo`):

```toml
# winit standalone
winit = "0.30"

# egui embedded
eframe    = { version = "0.36", default-features = false, features = ["wgpu", "x11", "wayland"] }
egui      = "0.36"
egui-wgpu = "0.36"
```

The X11/Wayland features above enable Linux window creation; select the Linux
window backends appropriate for your host. For iced, see
[iced integration status](#iced-integration-status).

### Editing and refreshing a chart

To edit a chart, change only the fields you need in its `Config`, the single
source of truth (SSOT) from which its view and exports are derived. For example,
write axis `min` / `max` through `Chart::with_axis_range_change` to preserve tick
settings. Convenience helpers such as `set_x_range`, `auto_fit_*` and `with_title`
also adjust related ticks, labels or title layout. See the
[operation table and range-only example](API.md#direct-ssot-edits-and-convenience-helpers)
before choosing an edit path.

For the full model-edit → prepare → pick → redraw → export sequence, including
the distinction between logical and physical pointer coordinates, see
[the native API workflow](API.md#native-edit-pick-and-export-workflow).

Cartesian hosts own the derived `ChartView` and, with `prepare`, the series'
`ChartStyle` values. After changing axes, grid, labels, legend or panel size, call
`refresh_axis` (or `refresh_axis_with_selection`) before preparing. After changing
a series' appearance, mapped styles or identity, recreate its `ChartStyle` with
`create_style_for_series` (or `create_style_for_series_scaled` at the display
scale the style was built for). `prepare` updates transforms and GPU data but
does not perform those two steps. Charts registered with `Renderer::register_chart`
keep their own style cache, updated by `set_chart_config` / `set_chart_series`
and drawn with `prepare_registered`; the caller-owned `ChartView` still needs
`refresh_axis`.
See the [edit table and compiled example](API.md#cartesian-edits-update-the-derived-inputs-before-preparing).

The refresh also reports tick settings the axis raster could not draw as written.
Read `view.render_warnings()` in the same branch that calls `refresh_axis`: each
`RenderWarning` names the axis, the value as written and the value drawn, such
as a `major_spacing` that would need more than 1000 intervals and was drawn at
automatic spacing. The list is re-derived only when the view is rasterized, so a
host that refreshes on its dirty flag never pays for it per frame. Compare it
with the previous list to show only changes. The same list ends with the
annotation layer's `AnnotationSkipped` warnings for items the annotation raster
could not place. After an edit of `config.annotations.items`, `refresh_axis`
redoes the annotation rasters together with the axes, and `refresh_annotations`
redoes only the annotation layer (its underlay and overlay textures). Registered
charts report that edit through `ChartRenderStamp::needs_annotation_raster_since`
(see [the annotation layer](INTERACTION.md#annotation-layer)).

Constructors initialize base resources; they do not eagerly populate every
pipeline cache. Use explicit `prewarm_all` or detached preparation when needed,
following the [constructor/prewarm table](API.md#construction-and-prewarming).
`set_memory_budget` is a policy for checked allocations, not a hard VRAM ceiling;
the [budget contract](API.md#gpu-memory-accounting-and-budget-scope) lists its
initial-pool behavior, tracked lifetime and exclusions.

### Standalone setup and drawing (winit + figgy)

The following initialization/draw fragment belongs in a winit
`ApplicationHandler::resumed` handler, with `event_loop: &ActiveEventLoop`.
See [winit_simple.rs](examples/winit_simple.rs) for the complete
event loop and persistent window/renderer state.

```rust
use std::sync::Arc;
use renderer::{
    Chart, ChartDrawItem, DataLineStyleConfig, DataRenderType, Renderer, Series, SeriesConfig,
    color::Color, default, layout::{ChartArea, Rect}, line::LineStylePreset,
};

let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()).unwrap());
let size = window.inner_size();

// One-line setup — figgy owns instance/adapter/device/queue/surface/swap chain.
let mut renderer = Renderer::for_window(
    Arc::clone(&window),
    (size.width, size.height),
    16 * 1024 * 1024,   // 16 MiB GPU column pool
).unwrap();

// renderer.add_column takes `&dyn ColumnSource`.
// Implement the trait on your own type (see `ColumnSource` section below) — Vec, ndarray,
// polars Series, mmap, anything. Built-in `Column<f64>` works too.
let xs: Vec<f64> = (0..1024).map(|i| i as f64 * 0.01).collect();
let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
fn column(data: Vec<f64>) -> renderer::Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    renderer::Column { data, min, max }
}
renderer.add_column("x", &column(xs)).unwrap();
renderer.add_column("y", &column(ys)).unwrap();

// Chart — builder pattern.
let mut config = default::default_config();
config.chart_area = ChartArea(Rect { x:8, y:8, width: size.width.saturating_sub(16).max(1), height: size.height.saturating_sub(16).max(1) });
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
            fill: None,
            fill_color: None,
            interpolation: renderer::data_config::LineInterpolation::Linear,
        },
    },
};
let style = renderer.create_style_for_series(&cfg).unwrap();   // budget-checked ChartStyle
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

`Renderer::add_column` takes `&dyn ColumnSource` — implement the trait on any container of yours and native upload writes into mapped staging without an intermediate conversion `Vec`. The GPU then copies staging into the column pool; “zero-copy” here means no extra CPU conversion buffer or retained CPU point mirror. In the same pass that writes the GPU pairs, the source returns `ColumnUploadStats::min_positive`, the smallest finite positive value it wrote; log-axis auto-fit uses it as the lower bound when the data contains zeros or negatives. The renderer cannot read wgpu 30's write-only mapped bytes back, so this statistic must come from the source. `min` / `max` retain their source-level meaning. For a scalar column `min_positive` is taken from the actual uploaded `(value as f32, 0)` value, and for a hi-lo column from the recorded `hi as f64 + lo as f64`; both include only finite positive values. Use `Renderer::add_hilo_column` with `&dyn HiLoColumnSource` for large absolute timestamps or coordinates that must preserve sub-f32 deltas.

`add_column` / `add_hilo_column` register a new id. To atomically replace an
existing id, use `upsert_column` / `upsert_hilo_column`. `Renderer` prepares the
provisional pool, affected chart revisions, active picker transition, and
maintenance state before publishing any of them. If any of that preparation
fails, the call returns the error and the previous column, chart state and
picker stay in effect. Integrations with additional
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

`Column<f64>`, `Column<f32>` and `Column<Option<f64>>` also implement
`HiLoColumnSource`; browser hosts should pass a
`Float64Array` through `register_column_f64` for the first upload and
`update_register_column_f64` for an explicit replacement when using timestamp
axes with large Unix epoch values.

Custom implementations of either trait must implement
`write_f32_pair_le_into_with_stats`. It has no default, so a source that omits
it fails to compile instead of failing during upload. There is no byte-readback
or silently incorrect fallback.

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

renderer.add_column("temperature", &my_series)?;   // my_series: MyTimeSeries
```

Native `f32` containers use the same fused path: iterate the values and call
`writer.write_pair(index, value, 0.0)`. `ColumnPairWriter` exposes indexed pair
writes rather than mapped bytes: the source writes straight into staging without
a per-value buffer and never sees wgpu types.

### Native examples — sine / RC / cross-section

```bash
cargo run -p figgy-renderer --example winit_simple
cargo run -p figgy-renderer --example egui_embed --features egui_demo
```

Each example shows:
- Three panels with different grid options (off / major / major + dotted minor);
  `winit_simple` adds a fourth panel with a heatmap and labelled contours
- Click selection of axes, tick labels, axis titles, chart titles, legends and the
  data area; dragging moves the selected element, and eight handles resize the
  data area
- The RC panel renders 2 series (charging + discharging)
- Line widths of 1 / 2 / 3.5 px across panels
- Legends
- DPI input + Save PNG button (egui) or `S` key (winit) → per-panel PNG bytes in memory → written by the example to `/tmp/figgy_*_panel_{i}.png`

### Browser timestamp axes demo

`crates/web/timestamp-demo.html` exercises the browser timestamp path with
absolute Unix time values uploaded through `register_column_f64(Float64Array)`
and explicitly replaced through `update_register_column_f64`.
It lets you change the visible time window, data unit, timezone, fractional
second policy, label pattern, chart width, and export scale while the x axis
uses `LabelFormat::Timestamp` + `AutoCalendar` to choose non-overlapping labels.

```bash
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
cd crates/web
python3 -m http.server 8142 --bind 127.0.0.1
# open http://127.0.0.1:8142/timestamp-demo.html
```

### Live SSoT lab — the split API at pool scale

```bash
cargo run --release -p figgy-renderer --example ssot_lab --features egui_demo
```

A 2×2 grid, one draw style per panel (Precise dashed / Sketch / Milkyway /
Constellation), all four series reading a single shared `x` pool column. The
sidebar edits the SSoT live — pan direction per x-linked column pair, window
width, and point density up to 3M/series (12M total, 5 columns). Every edit
flows through `Renderer::prepare` → `Renderer::paint_prepared` with no
`Mutex` and no per-frame `update_transform`. The status box's `frames
skipped` counter counts panel frames that had no token or whose `paint_prepared`
call failed, so it shows whether a stale token made a panel skip a frame during
an edit.

### egui integration pattern (summary)

The frame is split to match host callback shapes: every mutation lives in
`Renderer::prepare` (`&mut self`), pure command recording lives in
`Renderer::paint_prepared` (`&self`) — so the paint callback needs no
`Mutex` around the renderer:

```rust
// Sketch: parameter types are elided; see examples/egui_embed.rs for the full code.
// stored in CallbackResources as plain FiggyState
struct FiggyState { renderer: renderer::Renderer, panels: Vec<PanelEntry> }
struct PanelEntry { /* chart, view, … */ prepared: Option<renderer::PreparedFrame> }
struct FiggyCallback { panel_idx: usize /* one callback per panel */ }

impl egui_wgpu::CallbackTrait for FiggyCallback {
    fn prepare(&self, _device, _queue, _screen, _enc, resources) -> Vec<...> {
        let state = resources.get_mut::<FiggyState>().unwrap();
        // dirty handling: refresh_axis (raster). No per-frame update_transform —
        // prepare writes the transform uniform itself.
        // items = panel_draw_items(panel, &series): a free function over the panel
        // fields, so `state.renderer` stays free for this `&mut` call.
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
Contour labels (automatic or at explicit anchors) and the arc-length and
star-placement compute results follow the same ownership rule: each distinct input gets
its own immutable GPU result, and a later `prepare` never rewrites a result an
earlier token recorded, even after that token is dropped, so a command buffer
recorded from it stays valid until submission. Paint uses only the token's
resources and never re-reads the series cache.

The host must submit a command buffer recorded from a token before the next
mutation of that token's `ChartView` (`refresh_axis`, `update_transform`, or a
later `prepare` using the same view). The content revision rejects stale input
before recording; it cannot inspect or order a command buffer after ownership
has moved to the host.
The one-shot `Renderer::paint(&mut self, …)` facade remains for hosts that own
the renderer exclusively during their frame (winit loop, wasm wrapper): it
runs both phases back to back.

Full version: [examples/egui_embed.rs](examples/egui_embed.rs). That example
does not call `end_gpu_frame`; [tools/playground/src/main.rs](https://github.com/shim9610/figgy/blob/master/tools/playground/src/main.rs)
is a host that calls it once after `queue.submit`.

### iced integration status

The retained [iced integration source](https://github.com/shim9610/figgy/blob/master/crates/renderer/unsupported/iced_embed_wgpu27.rs)
documents the intended `prepare` / `paint_prepared` ownership pattern, but its
build target is disabled while iced 0.14 remains on wgpu 27. wgpu device,
queue, and render-pass types cannot be shared across major versions.

### Cartesian series, maps and layers

Every column-backed series is declared in `SeriesConfig::render_type`
([series schema](CONFIG.md#data_config--declarative-series-schema-the-active-api)).
Beyond lines, scatter, error bars, histograms, heatmaps and contours:

- **Area fills and step lines**: a line's `fill` paints to a baseline or to
  another series sharing its x column, and `interpolation` draws `StepPre`,
  `StepPost` or `StepMid` pieces that dashes and picking follow.
  [Reference](CONFIG.md#area-fill-and-step-lines) and `area_step`.
- **Point colour and size maps**: a scatter series colours its points by a
  value column through the chart's colourbar, or sizes them over an explicit
  domain (a bubble chart).
  [Reference](CONFIG.md#point-colour-and-size-maps) and `value_maps`.
- **Vector fields**: `DataRenderType::Vector` draws one arrow per sample from
  (u, v) columns at scattered anchors or from two matrices on a grid, in
  pixel or data scale, optionally coloured through the colourbar.
  [Reference](CONFIG.md#vector-fields) and `vector_field`.
- **Overlay axes, subplots and annotations**: each overlay layer draws its series
  against its own right-hand Y axis; subplots arrange whole charts in a grid with
  linked or twin axes; the annotation layer adds reference lines, spans,
  text and arrows. Polar charts (radar charts included) and radial charts have no value axes: they draw
  ratio-anchored text and arrows only and skip reference lines, spans and
  data-anchored items with `RenderWarning::AnnotationSkipped`. A document has
  one annotation layer, drawn in the parent chart's area; subplots have none of
  their own. [Overlays](INTERACTION.md#overlay-layers),
  [Subplots](INTERACTION.md#subplots), [Annotations](INTERACTION.md#annotation-layer);
  `overlay_axes` and `subplots`.

```bash
cargo run --locked -p figgy-renderer --example area_step -- target/area-step
cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field
cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes
cargo run --locked -p figgy-renderer --example subplots -- target/subplots
```

### Box plots

A `DataRenderType::BoxPlot` series draws precomputed quartiles, medians,
whiskers, means and outliers over a categorical axis, read from a record of box
statistics and a ragged outlier column, with matte/satin surfaces, individual
styles, grouped layouts and either direction. Notches use explicit median
confidence intervals. `Config::from_boxplot` opens a legacy `BoxPlotChart`
document. See the [box plot guide](BOXPLOT.md) or run
`cargo run -p figgy-renderer --example boxplot_gallery`. Picking, selection,
titles, axes, labels and legends are the Cartesian chart's
([interaction guide](INTERACTION.md)).

### Radar charts

A radar chart is a polar chart with a categorical angle axis: one spoke per
category, one shared numeric radius axis, and `DataRenderType::Radar` series
drawn as closed outlines with fills and vertex markers on a polygon
(`PolarOptions.grid_shape`) or circular grid. The host supplies one value per
category and series as columns; per-spoke normalization is not computed.
`Config::from_radar` opens a legacy `RadarChart` document. See the
[radar guide](RADAR.md) or run
`cargo run -p figgy-renderer --example radar_gallery -- target/radar-gallery`.
Picking, selection, titles, axes, labels and legends are the polar chart's
([interaction guide](INTERACTION.md)).

### Polar plots

A polar plot is a Cartesian chart with `config.polar = Some(PolarOptions { .. })`:
the same line, scatter and area series, column pool, picking and export, drawn
in (angle, radius). `bottom_x` is the angle axis and `left_y` the radius axis.
`set_y_range` sets the radius range; `set_x_range` only chooses which angle
ticks, spokes and labels are drawn, because one turn always spans 360° (or 2π)
from `zero_angle_degrees`. The grid becomes rings and spokes, the angle axis the
outer ring and the radius axis a labelled ray that drags around the centre.
Angles are degrees or radians, the circle can be a sector with a hole, and
segments are chords or arcs. Series without a polar form (histograms, heatmaps,
contours, error bars, vectors), overlay layers, step lines under
`PolarInterpolation::Arc` and streamed sources are rejected with an explicit
error. See the [polar guide](POLAR.md) or run
`cargo run -p figgy-renderer --example polar_plot -- target/polar-plot`.

### Categorical bars

The Cartesian `Renderer` draws grouped, stacked and 100% stacked bars in
either direction as `DataRenderType::CategoryBars` series over a categorical
axis, laid out by `Config.bar_groups`. Their positions and values are columns
uploaded through `ColumnSource`; streamed sources are rejected. Per-bar
colours, materials, rounding and outlines are per-bar overrides on the
series, and a bar is picked and selected like a histogram bin.
`Config::from_categorical` opens a legacy `CategoricalChart` document as such
a chart. See the [bar guide](CATEGORICAL.md) or run
`cargo run -p figgy-renderer --example categorical_gallery -- target/categorical-gallery`.

### Radial charts

`RadialRenderer` draws pie, donut, and pie-of-pie charts from the slices held in a
`RadialChart`, with procedural matte, ceramic, satin/brushed-metal, toon, and other
surfaces. Rounded sector corners, outlines, hover/selection, per-slice colors and
styles, and label content are fields of that model. It shares the host wgpu device
and does not use the column pool or column streams.
Try the [native editor](examples/radial_editor.rs) with
`cargo run -p figgy-renderer --example radial_editor --features egui_demo`.
The [radial chart guide](RADIAL.md) (in Korean) covers the API with an example,
[option defaults and ranges](RADIAL.md#스타일-옵션-기본값과-범위),
[per-slice editing](RADIAL.md#ssot-편집과-선택), and
[host update order](RADIAL.md#호스트의-갱신-순서), including frame ownership and export.

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
In the snippet, `series_configs` holds the `SeriesConfig` values to draw (for the
setup example above, `std::slice::from_ref(&cfg)`) and `scale` is the export
scale, such as `2.0`. The image is the chart area times `scale` on a fully
transparent background. The `_with_clear_async` variants take a background
colour as a fourth argument, for example
`export_panel_png_bytes_with_clear_async(&chart, &series_configs, scale, Color::WHITE)`.
`export_panel_*` draws the base chart area only. For a chart with `overlays` or
`subplots`, `export_document_png_bytes_async` / `export_document_rgba_async`
(and their `_with_clear` and blocking variants) take the same arguments and
draw every layer and subplot into one image covering all chart areas.
`scale` is at least `renderer::MIN_EXPORT_SCALE` (0.25; NaN and values at or below zero become it) and has no upper bound: the scaled image must fit the device's texture limit (`max_texture_dimension_2d`, commonly 8192 or 16384 px per side), and a larger export fails with `FiggyError::GpuResourceLimit` before anything is allocated.
Convert from standard 96 DPI via `renderer::dpi_to_scale(dpi)`.

When scaling, every pixel-based dimension (font, line, margin, grid, legend)
scales by the same factor, so the layout matches the on-screen chart at a
higher pixel density (or a lower one below 1.0).

### SVG and PDF export

```rust
let svg: String = renderer
    .export_document_svg_async(&chart, &series_configs, scale)
    .await?;
let pdf: Vec<u8> = renderer
    .export_document_pdf_async(&chart, &series_configs, scale)
    .await?;
std::fs::write("/tmp/out.svg", svg)?;
std::fs::write("/tmp/out.pdf", pdf)?;
```

The two documents hold the same picture as `export_document_png_bytes_async`
with the same arguments ([#48](https://github.com/shim9610/figgy/issues/48)).
The decorations are vector content, written from the placement the PNG
draws with: axes, ticks, tick labels, grid, polar rings and spokes, axis and
chart titles, legend, the colourbar's border, axis and labels, and the
annotation layer. Text is written as glyph outlines, so no font is embedded
and the text cannot be selected or searched. The data layer (fills, lines,
markers, bars, fields, contours, vectors, picked highlights and value
labels) is drawn by the GPU, so each panel group's data is one image at
`scale ×` resolution, the PNG export's own pixels: one image for a chart and
its overlays, one more for each subplot. The colourbar ramp and the
Milkyway backdrop are images too; the Milkyway glow is not drawn. Selection
boxes are never drawn, as in the PNG export.

The SVG's `viewBox` is in physical pixels and its `width` / `height`, like
the PDF page, are the logical size in points (1 px = 0.75 pt), so a 2x export
has the same page size with twice the image resolution. `scale` is clamped
as for the PNG export, the `_with_clear_async` variants paint the clear
colour as a rectangle under everything, and the native blocking wrappers are
`export_document_svg` / `export_document_pdf`. `WindowedRenderer` has the same
six methods. A series with a streamed source is rejected with
`FiggyError::ColumnNotResident`, as by the PNG export.

The radial chart exports with `RadialRenderer::export_svg_async(&chart, size,
scale)` and `export_pdf_async` (blocking: `export_svg` / `export_pdf`): the
slices and background are one image, the title, labels, leader lines, split
connectors and annotations are vector content. Every inside label is drawn
above every slice at its place in the PNG, and no selection box is drawn.

---

---

## 1. 사용법

### 툴체인과 빌드

[`rust-toolchain.toml`](https://github.com/shim9610/figgy/blob/master/rust-toolchain.toml)에 개발 환경을 **Rust 1.99.0**으로 고정하고 wasm32 빌드 대상, rustfmt, clippy를 지정했다. rustup이 설치돼 있으면 저장소 안에서 명령을 실행할 때 자동으로 적용된다. 패키지 manifest에도 지원 하한을 Rust 1.99로 명시했다. 그보다 낮은 버전은 검증하지 않았다.
이 저장소를 빌드할 때는 `--locked`를 사용해 커밋된 `Cargo.lock`을 따른다. figgy를 의존성으로 사용하는 프로젝트는 해당 프로젝트의 잠금 파일로 의존성을 결정한다.

```bash
cargo check --locked --workspace --all-targets --all-features
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
```

### 의존성 추가

Cargo 패키지 이름은 `figgy-renderer`·`figgy-model`이고 Rust 라이브러리 이름은
`renderer`·`model`이다. 코드에서는 `renderer::…`·`model::…`로 가져오며, 의존성에는
아래처럼 `package` 키로 패키지 이름을 지정한다.

렌더러의 `serde` 기능을 켜면 재노출된 모델 타입을 직렬화할 수 있다. 이를 위해 모델을
직접 의존할 필요는 없다. GPU를 직접 공유하려면 호스트도 wgpu 30을 사용해야 한다.
브라우저용 제품인 WASM 래퍼(`figgy`)는 별도 패키지이며 `publish = false`다. 배포 대상은 [아직 정하지 않았다](https://github.com/shim9610/figgy/issues/57).
소유권·프레임 수명·출력 타깃·호환성은 [API 계약](API.md)을 참고한다.

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.18.1" }
wgpu     = "30"
```

이 설정은 crates.io에서 `figgy-renderer` 0.18.1을 받는다. 렌더러 없이 모델 타입만 쓰는
크레이트는 `model = { package = "figgy-model", version = "0.12.1" }`을 의존성으로 지정한다.

로컬 체크아웃을 쓰려면 `version` 대신 `path = "../figgy/crates/renderer"`처럼 애플리케이션의
`Cargo.toml` 기준 경로를 지정한다. [examples/line.rs](examples/line.rs)처럼
`pollster::block_on`을 직접 호출하는 독립 프로그램은 `pollster = "1.0"`도 의존성에 추가한다.
저장소의 예제는 렌더러 manifest를 통해 이 의존성을 이미 쓸 수 있다.

### 호스트 크레이트

기본 라이브러리 기능에는 winit·egui·iced가 필요하지 않다. 사용하는 호스트 크레이트만
추가한다. 이 크레이트의 egui 예제는 `--features egui_demo`로 실행한다.

```toml
# winit standalone
winit = "0.30"

# egui 임베드
eframe    = { version = "0.36", default-features = false, features = ["wgpu", "x11", "wayland"] }
egui      = "0.36"
egui-wgpu = "0.36"
```

위 egui 설정의 `x11`·`wayland` 기능은 Linux에서 창을 만들 때 쓴다. 호스트 환경에 맞는
Linux 창 백엔드만 고른다.

iced 0.14는 wgpu 27을 사용하므로, wgpu 30과 호환되는 iced 버전이 나올 때까지 장치·큐·렌더 패스를 직접 공유하는 통합은 사용할 수 없다([iced 통합 상태](#iced-통합-상태)).

### 차트 편집과 갱신

차트를 편집할 때는 `Config`에서 필요한 필드만 고친다. `Config`는 뷰와 내보내기 결과를
만드는 단일 원본(SSOT)이다. 예를 들어 축 `min` / `max`를 `Chart::with_axis_range_change`로
바꾸면 눈금 설정이 그대로 남는다. `set_x_range`, `auto_fit_*`, `with_title` 같은 편의 함수는
관련 눈금·라벨·제목 배치도 함께 조정한다. 편집 방법을 고르기 전에
[작업 표와 범위 편집 예제](API.md#direct-ssot-edits-and-convenience-helpers)를 확인한다.

모델 편집 → prepare → 피킹 → 다시 그리기 → 내보내기의 전체 순서와 논리·물리 포인터 좌표의
차이는 [네이티브 API 작업 흐름](API.md#native-edit-pick-and-export-workflow)에 있다.

직교 차트 호스트는 파생 상태인 `ChartView`를 보관하고, `prepare`를 쓸 때는 시리즈별
`ChartStyle`도 보관한다. 축·격자·라벨·범례·패널 크기를 바꾼 뒤에는 prepare 전에
`refresh_axis`(또는 `refresh_axis_with_selection`)를 호출한다. 시리즈의 모양·매핑 스타일·
식별자를 바꿨다면 `create_style_for_series`로 `ChartStyle`을 다시 만든다. 특정 표시 배율로
만든 스타일은 같은 배율로 `create_style_for_series_scaled`를 호출한다. `prepare`는 변환과
GPU 데이터만 갱신하며 이 두 단계를 대신하지 않는다. `Renderer::register_chart`로 등록한
차트는 자체 스타일 캐시를 두고 `set_chart_config` / `set_chart_series`로 갱신하며
`prepare_registered`로 그린다. 이때도 호출자가 가진 `ChartView`에는 `refresh_axis`가 필요하다.
[편집 표와 컴파일되는 예제](API.md#cartesian-edits-update-the-derived-inputs-before-preparing)를
참고한다.

`refresh_axis`는 축 래스터가 설정대로 그리지 못한 눈금 설정도 알려 준다. `refresh_axis`를
호출한 분기에서 `view.render_warnings()`를 읽는다. 각 `RenderWarning`에는 축, 설정한 값,
실제로 그린 값이 들어 있다. 예를 들어 간격이 1000개를 넘어야 하는 `major_spacing`은 자동
간격으로 그린다. 목록은 뷰를 래스터화할 때만 다시 만들므로 변경 플래그에 따라 갱신하는
호스트는 프레임마다 비용을 내지 않는다. 바뀐 경고만 보여 주려면 이전 목록과 비교한다.
목록 끝에는 주석 층이 배치하지 못한 항목의 `AnnotationSkipped` 경고가 붙는다.
`config.annotations.items`를 고친 뒤 `refresh_axis`를 호출하면 축과 함께 주석 래스터도 다시
그리고, `refresh_annotations`는 주석 층(아래층·위층 텍스처)만 다시 그린다. 등록 차트에서는
`ChartRenderStamp::needs_annotation_raster_since`로 이 편집을 확인한다
([주석 층](INTERACTION.md#annotation-layer)).

생성자는 기본 자원만 초기화하며 모든 파이프라인 캐시를 미리 채우지는 않는다. 필요하면
`prewarm_all`이나 분리 준비 경로를 명시적으로 사용한다
([생성자·사전 준비 표](API.md#construction-and-prewarming)). `set_memory_budget`은 검사를
거치는 할당에 적용하는 정책이며 VRAM의 절대 상한이 아니다. 초기 풀 처리, 추적 기간, 제외
항목은 [예산 계약](API.md#gpu-memory-accounting-and-budget-scope)에 있다.

<a id="standalone-초기화와-그리기-winit--figgy"></a>

### 독립 창에서 초기화하고 그리기 (winit + figgy)

다음 코드는 winit의 `ApplicationHandler::resumed` 메서드 안에서 실행하는 예다. `event_loop: &ActiveEventLoop`를 인자로 받는다. 전체 이벤트 루프와 창·렌더러 상태를 보관하는 방법은 [winit_simple.rs](examples/winit_simple.rs)를 참고한다.

```rust
use std::sync::Arc;
use renderer::{
    Chart, ChartDrawItem, DataLineStyleConfig, DataRenderType, Renderer, Series, SeriesConfig,
    color::Color, default, layout::{ChartArea, Rect}, line::LineStylePreset,
};

let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()).unwrap());
let size = window.inner_size();

// 한 줄로 초기화한다. instance·adapter·device·queue·surface·swap chain은 모두 figgy가 소유한다.
let mut renderer = Renderer::for_window(
    Arc::clone(&window),
    (size.width, size.height),
    16 * 1024 * 1024,   // GPU column pool 16 MiB
).unwrap();

// renderer.add_column은 `&dyn ColumnSource`를 받는다.
// 사용하는 데이터 타입에 트레이트를 구현하면(아래 `ColumnSource` 절 참고) Vec, ndarray,
// polars Series, mmap 등 어떤 데이터든 넘길 수 있다. 기본 제공 `Column<f64>`를 써도 된다.
let xs: Vec<f64> = (0..1024).map(|i| i as f64 * 0.01).collect();
let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
fn column(data: Vec<f64>) -> renderer::Column<f64> {
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    renderer::Column { data, min, max }
}
renderer.add_column("x", &column(xs)).unwrap();
renderer.add_column("y", &column(ys)).unwrap();

// Chart — 빌더 패턴.
let mut config = default::default_config();
config.chart_area = ChartArea(Rect { x:8, y:8, width: size.width.saturating_sub(16).max(1), height: size.height.saturating_sub(16).max(1) });
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
            fill: None,
            fill_color: None,
            interpolation: renderer::data_config::LineInterpolation::Linear,
        },
    },
};
let style = renderer.create_style_for_series(&cfg).unwrap();   // budget-checked ChartStyle
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

<a id="columnsource--데이터-어댑터-trait"></a>

### `ColumnSource` — 데이터 어댑터 트레이트

`Renderer::add_column`은 `&dyn ColumnSource`를 받는다. 사용 중인 데이터 타입에 이 트레이트를 구현하면 네이티브 환경에서 중간 변환용 `Vec` 없이 매핑된 스테이징 버퍼에 직접 기록할 수 있다. 스테이징 버퍼에서 컬럼 풀로 옮기는 작업은 GPU가 수행한다. CPU에는 변환용 버퍼나 점 데이터의 복사본을 따로 두지 않는다.

데이터 소스는 값을 GPU용 쌍으로 기록하면서 최소 양수 값도 함께 계산한다. 렌더러는 wgpu 30의 쓰기 전용 매핑 영역을 다시 읽지 않는다. `min` / `max`는 원본 데이터 기준을 유지한다. 최소 양수 값은 일반 컬럼에서는 실제 업로드한 `(value as f32, 0)`, hi/lo 컬럼에서는 기록한 `hi as f64 + lo as f64`를 기준으로 계산하며, 유한한 양수만 포함한다.

큰 Unix 시각이나 절대 좌표처럼 f32로는 작은 차이가 사라지는 값은 `Renderer::add_hilo_column`에 `&dyn HiLoColumnSource`를 넘겨 등록한다. 기본 제공 `Column` 타입 세 가지는 이 트레이트도 구현한다. 브라우저에서 큰 Unix 시각을 시간축에 쓸 때는 처음 등록할 때 `register_column_f64`, 교체할 때 `update_register_column_f64`에 `Float64Array`를 넘긴다.

`add_column` / `add_hilo_column`은 새 ID를 등록할 때 사용한다. 기존 ID의 데이터를 교체하려면 `upsert_column` / `upsert_hilo_column`을 사용한다. `Renderer`는 새 풀 상태, 영향을 받는 차트의 리비전, 피킹 캐시와 정리 작업을 모두 준비한 뒤 한꺼번에 반영한다. 준비 중 오류가 나면 기존 상태를 유지한다.

호스트가 별도의 파생 상태를 관리한다면 `begin_upsert_*`가 반환한 가드의 임시 풀을 참조해 그 상태도 미리 준비할 수 있다. 이후 `commit`을 호출하면 추가 할당이나 실패 가능 작업 없이 변경을 확정한다. 피킹 자원의 갱신은 렌더러가 담당한다.

```rust
pub trait ColumnSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }  // 디폴트 제공
    fn min(&self) -> f64;
    fn max(&self) -> f64;

    /// 기존 구현과의 소스 호환을 위해 남겨 둔 스칼라 인코더다.
    /// 호출자가 `dst.len() == self.len() * 4`를 보장한다. null은 `f32::NAN`으로 기록한다.
    fn write_f32_le_into(&self, dst: &mut [u8]);

    /// `(value as f32, 0)` 쌍을 기록하고 같은 순회에서 계산한 통계를 반환한다.
    fn write_f32_pair_le_into_with_stats(
        &self,
        writer: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats;
}
```

**기본 제공 구현체**: `Column<f64>`, `Column<f32>`, `Column<Option<f64>>` (null → NaN).

사용자 정의 `ColumnSource`는 값 기록과 통계 계산을 함께 수행하는 메서드를 반드시 구현해야 한다. 빠뜨리면 컴파일 단계에서 오류가 나므로 업로드 도중에야 문제를 발견하는 일을 피할 수 있다. 렌더러는 기록한 바이트를 다시 읽지 않으며, 이 메서드를 대신하는 부정확한 대체 경로도 없다. `HiLoColumnSource`도 같은 이름의 메서드에서 `(hi, lo)`를 기록하고, 두 값을 합쳐 계산한 통계를 반환해야 한다.

**사용자 정의 구현 예: 시계열·DataFrame·mmap·FFI 데이터**

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

renderer.add_column("temperature", &my_series)?;   // my_series: MyTimeSeries. 중간 Vec 없이 스테이징 메모리에 바로 기록한다
```

`f32` 데이터도 같은 경로를 사용한다. 값을 순회하며 `writer.write_pair(index, value, 0.0)`를 호출하면 된다. `ColumnPairWriter`는 매핑된 바이트를 노출하지 않고 인덱스로 값 쌍을 쓰는 기능만 제공하므로 `dst.copy_from_slice(...)` 같은 일괄 복사는 할 수 없다. 소스는 wgpu 타입을 다루지 않고 중간 버퍼 없이 스테이징 메모리에 바로 기록한다.

<a id="네이티브-example--사인--rc--cross-section"></a>

### 네이티브 예제 — 사인 곡선, RC 회로, 단면 그래프

```bash
cargo run -p figgy-renderer --example winit_simple
cargo run -p figgy-renderer --example egui_embed --features egui_demo
```

각 예제에서 다음 기능을 확인할 수 있다.

- 패널 3개에 서로 다른 격자 설정 적용: 격자 없음, 주 격자선, 주 격자선과 점선 보조 격자선.
  `winit_simple`에는 히트맵과 라벨이 붙은 등고선을 그린 네 번째 패널이 더 있다
- 클릭으로 축·눈금 라벨·축 제목·차트 제목·범례·데이터 영역 선택. 선택한 요소는 드래그로 옮기고,
  데이터 영역은 핸들 8개로 크기를 조정한다
- RC 회로 패널의 충전·방전 곡선 두 개
- 패널별 선 두께 1 / 2 / 3.5px와 범례
- egui의 DPI 입력·Save PNG 버튼 또는 winit의 `S` 키로 패널별 PNG 저장. 예제는 메모리로 받은 PNG를 `/tmp/figgy_*_panel_{i}.png`에 저장한다.
- winit 예제에서는 `A` 키로 축 프레임 프리셋을, `C` 키로 시리즈 색 조합을 차례로 바꾼다

<a id="브라우저-timestamp-축-데모"></a>

### 브라우저 시간축 데모

`crates/web/timestamp-demo.html`은 Unix 시간 값을 `register_column_f64(Float64Array)`로 등록하고 `update_register_column_f64`로 교체하는 예제다. 시간 범위, 데이터 단위, 시간대, 소수 초 표시, 라벨 형식, 차트 너비, 출력 배율을 바꿔 볼 수 있다. X축에는 `LabelFormat::Timestamp`와 `AutoCalendar`를 사용해 눈금 간격을 자동으로 조절한다.

```bash
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
cd crates/web
python3 -m http.server 8142 --bind 127.0.0.1
# http://127.0.0.1:8142/timestamp-demo.html 열기
```

<a id="라이브-ssot-lab--풀-스케일에서-본-분리-api"></a>

### 대용량 실시간 편집 예제 (`ssot_lab`)

```bash
cargo run --release -p figgy-renderer --example ssot_lab --features egui_demo
```

2×2 패널에 `Precise`(점선)·`Sketch`·`Milkyway`·`Constellation` 그리기 스타일을 하나씩 표시한다. 네 시리즈는 GPU 풀의 `x` 컬럼 하나를 공유한다. 사이드바에서 X축이 연동된 패널 쌍의 이동 방향, 표시 범위의 너비, 시리즈당 데이터 수를 바꿀 수 있다. 데이터 수는 시리즈당 최대 300만 개(총 1200만 개, 컬럼 5개)이며, 장치가 허용한 풀 크기가 작으면 상한도 낮아진다.

설정 변경은 `Renderer::prepare`와 `Renderer::paint_prepared`를 거쳐 화면에 반영된다. `Mutex`나 프레임마다 별도로 호출하는 `update_transform`은 필요하지 않다. 상태창의 `frames skipped`로 편집 중 무효화된 프레임 토큰 때문에 그리기를 건너뛰었는지 확인할 수 있다.

### egui 통합 패턴 (요약)

호스트의 콜백 구조에 맞춰 프레임 준비와 명령 기록을 나눈다. 상태 변경은 `Renderer::prepare`가 `&mut self`로 수행하고, `Renderer::paint_prepared`는 `&self`로 그리기 명령만 기록한다. 따라서 paint 콜백에서 렌더러를 `Mutex`로 감쌀 필요가 없다.

```rust
// 개요 코드다. 매개변수 타입은 생략했으며 전체 코드는 examples/egui_embed.rs에 있다.
// FiggyState를 Mutex 없이 CallbackResources에 그대로 저장한다
struct FiggyState { renderer: renderer::Renderer, panels: Vec<PanelEntry> }
struct PanelEntry { /* chart, view, … */ prepared: Option<renderer::PreparedFrame> }
struct FiggyCallback { panel_idx: usize /* 패널마다 콜백 하나 */ }

impl egui_wgpu::CallbackTrait for FiggyCallback {
    fn prepare(&self, _device, _queue, _screen, _enc, resources) -> Vec<...> {
        let state = resources.get_mut::<FiggyState>().unwrap();
        // 패널 영역이 바뀌었거나 래스터를 다시 그려야 하면 refresh_axis를 호출한다.
        // prepare가 transform uniform을 직접 쓰므로 프레임마다 update_transform을 호출할 필요는 없다.
        // items = panel_draw_items(panel, &series): 패널 필드만 받는 자유 함수라서
        // 이 `&mut` 호출에 `state.renderer`를 그대로 쓸 수 있다.
        let prepared = state.renderer.prepare(&items).unwrap();
        // 토큰은 패널마다 따로 저장한다. egui는 모든 콜백의 prepare를 실행한 뒤 paint를
        // 실행하므로, 토큰 하나를 공유하면 뒤 패널의 prepare가 앞 패널의 토큰을 덮어쓴다
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

호스트는 한 프레임의 명령 버퍼를 모두 GPU 큐에 제출한 뒤 `renderer.end_gpu_frame()`을 한 번 호출한다. egui처럼 콜백 실행을 관리하는 호스트에서는 이전 프레임의 제출이 끝났음을 확인한 뒤 다음 프레임의 첫 prepare 직전에 호출해도 된다. 패널마다 호출하면 아직 제출하지 않은 다른 패널의 자원을 너무 일찍 회수 대상으로 넘길 수 있다.

`WindowedRenderer::draw*`나 패널 내보내기(`export_panel_*`)처럼 렌더러가 직접 제출하는 경로에서는 이 처리를 내부에서 수행한다. 외부 렌더 패스 기록과 이 경로를 함께 쓴다면 미리 기록해 둔 명령 버퍼를 먼저 모두 제출해야 한다. 렌더러는 wgpu 명령 버퍼의 내부 참조를 볼 수 없으므로 호스트가 아직 제출하지 않은 자원만 따로 골라 보호할 수 없다.

`paint_prepared`는 같은 토큰으로 여러 패스에 호출할 수 있다. `PreparedFrame`이 그리기에 필요한 입력을 보관하므로 paint 단계에서 `items`를 다시 만들거나 전달하지 않는다. 준비 후 관련 렌더러 자원이 바뀌면 아무 명령도 기록하지 않고 `FiggyError::StalePreparedFrame`을 반환한다. 다음 프레임에서 다시 `prepare`하면 된다.

등고선 라벨(자동 배치와 직접 지정한 앵커 모두)과 경로 길이·별 위치 계산 결과도 같은 소유권 규칙을 따른다. 입력마다 변경하지 않는 GPU 결과를 따로 만들고, 이후의 `prepare`는 앞선 토큰이 기록한 결과를 그 토큰이 해제된 뒤에도 덮어쓰지 않는다. 따라서 그 토큰으로 기록한 명령 버퍼는 제출할 때까지 유효하다. paint 단계는 토큰이 가진 자원만 쓰며 시리즈 캐시를 다시 조회하지 않는다.

토큰으로 기록한 명령 버퍼는 그 토큰의 `ChartView`를 다시 바꾸기 전에 제출해야 한다. `refresh_axis`, `update_transform`, 같은 뷰로 실행하는 다음 `prepare`가 여기에 해당한다. 렌더러는 기록하기 전에 콘텐츠 리비전으로 오래된 입력을 거부하지만, 호스트에 넘어간 명령 버퍼는 검사하거나 순서를 정할 수 없다.

winit 루프나 WASM 래퍼처럼 프레임 동안 렌더러를 단독으로 사용하는 호스트는 두 단계를 연속 실행하는 `Renderer::paint(&mut self, …)`를 사용하면 된다. 전체 예제는 [egui_embed.rs](examples/egui_embed.rs)를 참고한다. 이 예제는 `end_gpu_frame`을 호출하지 않는다. `queue.submit` 뒤에 한 번 호출하는 호스트는 [tools/playground/src/main.rs](https://github.com/shim9610/figgy/blob/master/tools/playground/src/main.rs)에서 볼 수 있다.

### iced 통합 상태

[보관 중인 iced 통합 예제](https://github.com/shim9610/figgy/blob/master/crates/renderer/unsupported/iced_embed_wgpu27.rs)에서 `prepare` / `paint_prepared`의 소유권 패턴을 확인할 수 있다. 다만 iced 0.14는 wgpu 27을 사용하므로 현재 빌드 대상에서는 제외한다. 서로 다른 wgpu 주 버전의 장치·큐·렌더 패스 타입은 공유할 수 없다.

### 직교 시리즈·매핑·레이어

컬럼 기반 시리즈는 모두 `SeriesConfig::render_type`으로 선언한다([시리즈 선언](CONFIG.md#data_config--시리즈-선언-형식)).
선·산점도·오차 막대·히스토그램·히트맵·등고선 외에 다음을 지원한다.

- **면 채움과 계단선**: 선의 `fill`은 기준선 또는 같은 x 컬럼을 쓰는 다른 시리즈까지 칠하고,
  `interpolation`은 `StepPre`·`StepPost`·`StepMid` 계단 조각을 그린다. 대시와 피킹도 그 경로를
  따른다. [설정](CONFIG.md#면-채움과-계단선)과 `area_step` 예제.
- **점 색·크기 매핑**: 산점도의 점을 값 컬럼으로 색상 막대를 거쳐 색칠하거나, 명시한 범위로
  크기를 정한다(버블 차트). [설정](CONFIG.md#점-색과-크기-매핑)과 `value_maps` 예제.
- **벡터장**: `DataRenderType::Vector`는 흩어진 기준점의 (u, v) 컬럼 또는 격자 위 행렬 두 개에서
  표본마다 화살표 하나를 픽셀 또는 데이터 스케일로 그리고, 색상 막대로 색칠할 수 있다.
  [설정](CONFIG.md#벡터장)과 `vector_field` 예제.
- **보조 Y축·서브플롯·주석**: 오버레이 레이어는 두 번째 Y축에 시리즈를 그리고, 서브플롯은 차트
  여러 개를 연결 축·쌍둥이 축과 함께 격자로 배치하며, 주석 층은 기준선·구간·텍스트·화살표를
  더한다. 극좌표(레이더 포함)·원형 차트에는 값 축이 없으므로 비율 좌표로 둔 텍스트와 화살표만 그리고,
  기준선·구간과 데이터 좌표 항목은 `RenderWarning::AnnotationSkipped` 경고를 내고 건너뛴다.
  주석 층은 문서마다 하나이며 부모 차트 영역에 그린다. 서브플롯에는 따로 없다.
  [오버레이](INTERACTION.md#overlay-layers), [서브플롯](INTERACTION.md#subplots),
  [주석](INTERACTION.md#annotation-layer)과 `overlay_axes`·`subplots` 예제.

```bash
cargo run --locked -p figgy-renderer --example area_step -- target/area-step
cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field
cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes
cargo run --locked -p figgy-renderer --example subplots -- target/subplots
```

### 박스플롯

`DataRenderType::BoxPlot` 시리즈는 범주형 축 위에 미리 집계한 사분위수·중앙값·수염·평균·이상치를
상자 통계 레코드와 이상치 가변 길이 컬럼에서 읽어 그린다.
가로·세로 및 그룹 배치, 무광·금속 질감, 개별 색·스타일과 선택을 지원한다.
노치는 입력받은 중앙값 신뢰구간으로 그린다. 예전 `BoxPlotChart` 문서는 `Config::from_boxplot`으로 연다.
[사용법](BOXPLOT.md)을 참고하거나 `cargo run -p figgy-renderer --example boxplot_gallery`로 예제를 실행한다.
선택과 제목·축·라벨·범례 편집은 직교 좌표 차트와 같다([공통 선택·이동 API](INTERACTION.md)).

### 레이더 차트

레이더 차트는 각도 축이 범주형인 극좌표 차트다. 범주마다 스포크 하나, 숫자 반지름 축 하나를
공유하며, `DataRenderType::Radar` 시리즈는 채움·외곽선·꼭짓점 마커가 있는 닫힌 외곽선이다. 고리는
다각형(거미줄, `PolarOptions.grid_shape`) 또는 원으로 그린다. 범주·시리즈마다 값 하나를 호스트가
컬럼으로 넣으며 스포크별 정규화는 계산하지 않는다. 예전 `RadarChart` 문서는 `Config::from_radar`로 연다.
[사용법](RADAR.md)을 참고하거나
`cargo run -p figgy-renderer --example radar_gallery -- target/radar-gallery`로 예제를 실행한다.
선택과 제목·축·라벨·범례 편집은 극좌표 차트와 같다([공통 선택·이동 API](INTERACTION.md)).

### 극좌표 플롯

극좌표 플롯은 `config.polar = Some(PolarOptions { .. })`를 켠 직교 차트다. 직교 차트와 같은
선·산점도·면 시리즈, 컬럼 풀, 피킹, 내보내기를 그대로 쓰고 좌표만 (각도, 반지름)으로 바꿔 그린다.
`bottom_x`가 각도 축, `left_y`가 반지름 축이다. `set_y_range`는 반지름 범위를 정한다. 원은 항상
`zero_angle_degrees`에서 시작하는 한 바퀴(360° 또는 2π)이므로 `set_x_range`는 그릴 각도
눈금·스포크·라벨만 정한다. 격자는 고리와 스포크로, 각도 축은 바깥 고리로 그린다. 반지름 축은 중심에서 뻗은 반직선에 눈금 라벨을 붙여 그리며, 드래그하면 중심을 기준으로
돌아간다. 각도 단위는 도 또는 라디안, 원은 구멍이 있는 섹터일 수 있고, 세그먼트는 현 또는 호다.
극좌표 형태가 없는 시리즈(히스토그램·히트맵·등고선·오차 막대·벡터), 오버레이 레이어,
`PolarInterpolation::Arc`에서의 계단선, 스트리밍 소스는 명시적 오류로 거부한다.
[사용법](POLAR.md)을 참고하거나
`cargo run -p figgy-renderer --example polar_plot -- target/polar-plot`로 예제 PNG를 만든다.

### 범주형 막대

범주형 막대는 직교 좌표 `Renderer`가 범주형 축 위에 그리는 `DataRenderType::CategoryBars` 시리즈다.
가로·세로 묶음, 누적, 100% 누적은 `Config.bar_groups`로 정하고, 위치와 값은 `ColumnSource`로 올린
컬럼에서 읽는다. 예전 `CategoricalChart` 문서는 `Config::from_categorical`로 연다.
[API 사용법](CATEGORICAL.md)을 참고하거나
`cargo run -p figgy-renderer --example categorical_gallery -- target/categorical-gallery`로 예제 PNG를 만든다.

### 원형 차트

`RadialRenderer`는 파이·도넛 차트와, 한 조각을 옆의 작은 원으로 펼치는 분할 원(pie-of-pie)을
그린다. 무광·세라믹·새틴 금속·툰 등의 재질, 둥근 모서리, 외곽선, 호버·선택을 지원한다.
조각별 색·스타일·표기 내용은 `RadialChart` 모델에서 직접 편집한다. 소수의 범주 데이터를 한 번에
받아 그리며 호스트의 wgpu 장치를 공유한다.
[네이티브 편집기](examples/radial_editor.rs)는
`cargo run -p figgy-renderer --example radial_editor --features egui_demo`로 실행한다.
[사용법과 예제](RADIAL.md)에서 [옵션 기본값과 범위](RADIAL.md#스타일-옵션-기본값과-범위),
[조각별 편집과 상속](RADIAL.md#ssot-편집과-선택), 프레임 수명과 내보내기를 포함한
[호스트 갱신 순서](RADIAL.md#호스트의-갱신-순서)를 확인할 수 있다.

<a id="png-export-메모리-only--저장은-caller"></a>

### PNG 내보내기

```rust
let bytes = renderer
    .export_panel_png_bytes_async(&chart, &series_configs, scale)
    .await?;
std::fs::write("/tmp/out.png", &bytes)?;          // 클립보드나 네트워크로 보내도 된다.

// RGBA만 필요하면:
let img = renderer
    .export_panel_rgba_async(&chart, &series_configs, scale)
    .await?;
// img.width, img.height, img.rgba (straight alpha, 길이 = w * h * 4)
```

네이티브에서는 이름에서 `_async`를 뺀 동기 래퍼도 제공한다. 파일 저장은 호출자가 담당한다.
예제의 `series_configs`는 그릴 `SeriesConfig` 슬라이스이고(앞의 초기화 예제라면
`std::slice::from_ref(&cfg)`), `scale`은 `2.0` 같은 출력 배율이다. 결과 이미지는 차트 영역에
`scale`을 곱한 크기이며 배경은 완전히 투명하다. `_with_clear_async` 버전은 네 번째 인자로 배경색을
받는다. 예: `export_panel_png_bytes_with_clear_async(&chart, &series_configs, scale, Color::WHITE)`.
`export_panel_*`는 기본 차트 영역만 그린다. `overlays`나 `subplots`가 있는 차트는 같은 인자를 받는
`export_document_png_bytes_async` / `export_document_rgba_async`(`_with_clear` 버전과 동기 버전 포함)로
레이어와 서브플롯을 이미지 한 장에 모두 그린다. 이 이미지는 모든 차트 영역을 합친 크기다.
`scale`의 최솟값은 `renderer::MIN_EXPORT_SCALE`(0.25)이고 NaN이나 0 이하의 값은 0.25로 바꾼다. 상한은 없다. 배율을 적용한 이미지가 장치의 텍스처 크기 한도(`max_texture_dimension_2d`, 보통 한 변 8192 또는 16384픽셀) 안에 들어와야 하며, 넘으면 메모리를 잡기 전에 `FiggyError::GpuResourceLimit` 오류를 돌려준다. DPI에서 배율을 구하려면 표준값 96 DPI를 기준으로 계산하는 `renderer::dpi_to_scale(dpi)`를 사용한다.

폰트, 선, 여백, 격자, 범례의 픽셀 크기가 모두 같은 비율로 바뀌므로 화면과 같은 배치를 더 높은 해상도로 얻는다. 1.0보다 작은 배율은 해상도를 낮춘다.

### SVG와 PDF 내보내기

```rust
let svg: String = renderer
    .export_document_svg_async(&chart, &series_configs, scale)
    .await?;
let pdf: Vec<u8> = renderer
    .export_document_pdf_async(&chart, &series_configs, scale)
    .await?;
std::fs::write("/tmp/out.svg", svg)?;
std::fs::write("/tmp/out.pdf", pdf)?;
```

두 문서에는 같은 인자로 `export_document_png_bytes_async`를 호출했을 때와 같은 그림이 담긴다([#48](https://github.com/shim9610/figgy/issues/48)).
장식 요소는 PNG를 그릴 때와 같은 배치에서 벡터로 기록한다. 축, 눈금, 눈금 라벨, 격자, 극좌표의 원과 방사선, 축 제목과 차트 제목, 범례, 색상 막대의 테두리·축·라벨, 주석 층이 여기에 해당한다. 글자는 글리프 윤곽선으로 기록하므로 글꼴을 포함하지 않으며, 글자를 선택하거나 검색할 수 없다.
데이터 층(채움, 선, 마커, 막대, 필드, 등고선, 벡터, 선택 강조, 값 라벨)은 GPU가 그리므로 패널 묶음마다 `scale`배 해상도의 이미지 한 장으로 넣는다. 이미지의 픽셀은 PNG 출력의 픽셀과 같다. 차트와 그 오버레이가 이미지 한 장이고, 서브플롯마다 한 장씩 더해진다. 색상 막대의 색 띠와 Milkyway 배경도 이미지로 넣으며, Milkyway의 발광 효과는 그리지 않는다. PNG 출력과 마찬가지로 선택 상자는 그리지 않는다.

SVG의 `viewBox`는 실제 픽셀 단위이고 `width`·`height`는 PDF 페이지처럼 논리 크기를 포인트로 나타낸다(1 px = 0.75 pt). 따라서 2배 출력은 페이지 크기가 같고 이미지 해상도만 두 배가 된다. `scale`은 PNG 출력과 같은 범위로 제한한다. `_with_clear_async` 버전은 배경색 사각형을 맨 아래에 그리고, 네이티브 동기 래퍼의 이름은 `export_document_svg` / `export_document_pdf`다. `WindowedRenderer`에도 같은 메서드 여섯 개가 있다. 스트리밍 원본을 쓰는 시리즈는 PNG 출력처럼 `FiggyError::ColumnNotResident`로 거부한다.

원형 차트는 `RadialRenderer::export_svg_async(&chart, size, scale)`와 `export_pdf_async`(동기 버전 `export_svg` / `export_pdf`)로 내보낸다. 조각과 배경은 이미지 한 장이고, 제목, 라벨과 그 지시선, 분할 원을 잇는 선, 주석은 벡터로 기록한다. 내부 라벨은 모두 PNG에서와 같은 위치에 모든 조각 위로 그리며, 선택 상자는 그리지 않는다.

---
