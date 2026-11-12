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
application's `Cargo.toml`. A native program needs no async executor: every call
that waits for the GPU has a `_blocking` form (see
[GPU results](#gpu-results-awaited-blocking-or-checked)), as
[examples/line.rs](examples/line.rs) uses.

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
`ChartStyle` values. After changing axes, grid, labels, legend or panel size
through the `Chart`, just prepare: `prepare` redraws the view's text and axis
raster when the chart changed since the view was drawn, with the element
`chart.select(..)` names drawn selected. `refresh_axis` (or
`refresh_axis_with_selection`) is for a host that redraws the raster itself.
After changing a series' appearance, mapped styles or identity, recreate its
`ChartStyle` with `create_style_for_series` (or `create_style_for_series_scaled`
at the display scale the style was built for); `prepare` does not rebuild it.
Charts registered with `Renderer::register_chart` keep their own style cache,
updated by `set_chart_config` / `set_chart_series`, and `prepare_registered`
redraws the caller-owned `ChartView` from the stored chart and its selection.
See the [edit table and compiled example](API.md#cartesian-edits-update-the-derived-inputs-before-preparing).

A redraw also reports tick settings the axis raster could not draw as written.
Read `view.render_warnings()` after a prepare that redrew the view: each
`RenderWarning` names the axis, the value as written and the value drawn, such
as a `major_spacing` that would need more than 1000 intervals and was drawn at
automatic spacing. The list is re-derived only when the view is rasterized,
never per frame. Compare it
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
let mut renderer = Renderer::for_window_blocking(
    Arc::clone(&window),
    (size.width, size.height),
    16 * 1024 * 1024,   // initial GPU column pool, 16 MiB; grows on demand
).unwrap();

// renderer.add_column takes `&dyn ColumnSource`. A `Vec` or slice of f64,
// f32, u32 or Option<f64> uploads as it is; implement the trait on your own
// type (see `ColumnSource` section below) for ndarray, polars Series, mmap, anything.
let xs: Vec<f64> = (0..1024).map(|i| i as f64 * 0.01).collect();
let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
renderer.add_column("x", &xs).unwrap();
renderer.add_column("y", &ys).unwrap();

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
    x_column: "x".into(), y_column: "y".into(), z_column: None,
    render_type: DataRenderType::Xy(renderer::data_config::XySeries { scatter: None, line: Some(DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::from_rgb8(20, 110, 230),
            line_width: 2.0,
            fill: None,
            fill_color: None,
            interpolation: renderer::data_config::LineInterpolation::Linear,
        }), errors: None, stem: None }),
};
let style = renderer.create_style_for_series(&cfg).unwrap();   // budget-checked ChartStyle
let mut view = renderer.create_chart_view(&chart, chart.config().chart_area.0).unwrap();

// frame loop:
let series = [Series { config: &cfg, style: &style }];
let mut items = [ChartDrawItem {
    view: &mut view,
    chart: &chart,
    series: &series,
}];
renderer.draw(Color::WHITE, &mut items).unwrap();   // acquire surface frame → prepare (redraws a stale raster) → encoder → pass → paint_prepared → submit → present
```

### `ColumnSource` — the data adapter trait

`Renderer::add_column` takes `&dyn ColumnSource`: implement the trait on any container of yours. The values are uploaded during the call, so the host may drop or change its container afterwards. A source only writes values. The `ColumnPairWriter` it writes through records the column's statistics in that same pass: the finite minimum and maximum and the smallest finite value above zero. `writer.write_value(index, v)` writes an f64 as its `(hi, lo)` split and records `v` itself; `writer.write_pair(index, hi, lo)` records `hi as f64 + lo as f64`. A value the GPU reads as zero (below f32 range) is recorded as zero. Auto-fit reads them, and log-axis auto-fit uses the smallest positive value as the lower bound when the data contains zeros or negatives. Non-finite values are left out. A streamed source is measured the same way, one chunk at a time. Use `Renderer::add_hilo_column` with `&dyn HiLoColumnSource` for large absolute timestamps or coordinates that must preserve sub-f32 deltas.

`add_column` / `add_hilo_column` register a new id. To replace an existing id
atomically, use `upsert_column` / `upsert_hilo_column`: if the replacement
fails, the call returns the error and the previous column, the charts that use
it and picking stay as they were. A host that keeps derived state of its own
can call `begin_upsert_*`, read the provisional pool through the guard's
`pool()`, prepare that state, and then call the guard's `commit`, which cannot
fail; dropping the guard instead keeps the previous column. The renderer
updates picking either way.

```rust
pub trait ColumnSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }  // default

    /// Write every value as a `(value as f32, 0)` pair (or a `(hi, lo)`
    /// split), each index in `0..len()` once. null → `f32::NAN`.
    fn write_pairs(&self, writer: &mut ColumnPairWriter<'_>);

    /// One streamed chunk: `writer.len()` values from `start`. Sources that
    /// are never streamed keep the default, which reports `Unsupported`.
    fn write_pair_range(
        &self,
        start: u64,
        writer: &mut ColumnPairWriter<'_>,
    ) -> Result<(), ColumnRangeWriteError>;
}
```

**Built-in implementors**: `Column<T>`, `Vec<T>`, `[T]` and `&[T]` for `T` = `f64`, `f32`, `u32` and `Option<f64>` (`None` → NaN). Arrays are not implementors: pass `&arr.as_slice()`.

```rust
pub trait HiLoColumnSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }

    /// Write every value as a `(hi, lo)` pair.
    fn write_pairs(&self, writer: &mut ColumnPairWriter<'_>);

    /// One streamed chunk, as for `ColumnSource`.
    fn write_pair_range(
        &self,
        start: u64,
        writer: &mut ColumnPairWriter<'_>,
    ) -> Result<(), ColumnRangeWriteError>;
}
```

The `f64`, `f32` and `Option<f64>` implementors also implement
`HiLoColumnSource`; browser hosts should pass a
`Float64Array` through `register_column_f64` for the first upload and
`update_register_column_f64` for an explicit replacement when using timestamp
axes with large Unix epoch values.

`write_pairs` has no default: every source implements it. The column's
statistics always come from the values the source writes.

**Custom — time series / DataFrame / mmap / FFI data, anything**:

```rust
struct MyTimeSeries {
    samples: Vec<f64>,    // or Arc<[f64]>, ndarray::ArrayView, polars::Series, ...
}

impl renderer::ColumnSource for MyTimeSeries {
    fn len(&self) -> usize { self.samples.len() }
    fn write_pairs(&self, writer: &mut renderer::ColumnPairWriter<'_>) {
        debug_assert_eq!(writer.len(), self.samples.len());
        for (index, &sample) in self.samples.iter().enumerate() {
            writer.write_pair(index, sample as f32, 0.0);
        }
    }
}

renderer.add_column("temperature", &my_series)?;   // my_series: MyTimeSeries
```

A native `f32` container does the same: iterate the values and call
`writer.write_pair(index, value, 0.0)`. `ColumnPairWriter` takes values by
index, one pair at a time, and does not expose the mapped bytes, so a source
never handles wgpu types.

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
- In `winit_simple`, the `A` key cycles the axis frame presets and the `C` key
  the series colour cycles

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

### Live editing at scale (`ssot_lab`)

```bash
cargo run --release -p figgy-renderer --example ssot_lab --features egui_demo
```

A 2×2 grid, one draw style per panel (Precise dashed / Sketch / Milkyway /
Constellation), all four series reading a single shared `x` pool column. The
sidebar edits the chart's configuration live: pan direction per x-linked
column pair, window width, and point density up to 3M/series (12M total, 5
columns), lower when the device allows a smaller pool. Every edit flows
through `Renderer::prepare` → `Renderer::paint_prepared`; the host needs no
`Mutex` and no per-frame `update_transform` call. The status box's `frames
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
        // No raster or transform handling: prepare redraws a stale raster and
        // updates the transform itself. The panel lends its view
        // mutably and its chart and series immutably (disjoint fields), so
        // `state.renderer` stays free for this `&mut` call.
        let prepared = state.renderer.prepare(&mut items).unwrap();
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
The contour labels (automatic or at explicit anchors), computed arc lengths
and star placements a token recorded are not overwritten by a later `prepare`,
even after the token is dropped, so a command buffer recorded from it stays
valid until submission.

The host must submit a command buffer recorded from a token before the next
mutation of that token's `ChartView` (`refresh_axis`, `update_transform`, or a
later `prepare` using the same view). The renderer rejects stale input before
recording, but it cannot inspect or order a command buffer once the host holds
it.
The one-shot `Renderer::paint(&mut self, …)` is for hosts that own
the renderer exclusively during their frame (winit loop, wasm wrapper): it
runs both phases back to back.

Full version: [examples/egui_embed.rs](examples/egui_embed.rs). That example
does not call `end_gpu_frame`; [tools/playground/src/main.rs](https://github.com/shim9610/figgy/blob/master/tools/playground/src/main.rs)
is a host that calls it once after `queue.submit`.

### GPU results: awaited, blocking or checked

A pick, an extent, a radial pick, an export and `wait_submitted_work` read a
GPU result back. Each can be waited for in three ways with the same result;
only the wait differs.

| Wait | Calls | Where it waits |
|---|---|---|
| Awaited | `ticket.resolve().await`, `renderer.export_png(..).await`, `wait_submitted_work().await` | Native: the device bundle's poll thread polls the device and the future is `Pending` until the work completes. Browser: the browser polls the device. |
| Blocking (native) | `ticket.resolve_blocking()`, `export_png_blocking(..)`, `wait_submitted_work_blocking()` | The calling thread polls the device until the work completes. |
| Checked | `ticket.try_resolve()` | Never waits: `TryResolve::Ready(result)` once the work has completed, `TryResolve::Pending(ticket)` before, to check again on a later frame. |

`RendererDevice::new` starts one poll thread per device bundle on native,
shared by the bundle's clones, its renderers and every pending ticket. The
thread parks while nothing is pending and exits once all of them are gone. A
host that makes blocking calls only builds the bundle with
`RendererDevice::blocking`, which starts no thread; an awaited wait on it
fails with `FiggyError::AsyncWaitUnavailable` instead of never completing.
A ticket on a streamed series resolves by `resolve` only
(`GpuPickError::Streamed`).

In an immediate-mode UI a hover pick fits `try_resolve`: submit the pick on
a pointer move, keep the ticket next to the panel, and check it each frame.
No thread blocks and the ticket needs no lock:

```rust
// Sketch: `pending: Option<GpuPickTicket>` lives in the panel's state.
if let Some(ticket) = pending.take() {
    match ticket.try_resolve()? {
        renderer::TryResolve::Ready(hit) => hover = hit,
        renderer::TryResolve::Pending(ticket) => pending = Some(ticket),
    }
}
```

### iced integration status

The retained [iced integration source](https://github.com/shim9610/figgy/blob/master/crates/renderer/unsupported/iced_embed_wgpu27.rs)
documents the intended `prepare` / `paint_prepared` ownership pattern, but its
build target is disabled while iced 0.14 remains on wgpu 27. wgpu device,
queue, and render-pass types cannot be shared across major versions.

### Series constructors

With the `helpers` feature (on by default), each chart family has a
constructor on `SeriesConfig` that names its columns and draws with the
host style defaults. Chainable modifiers set the common fields; any other
field is set on the returned series directly. The result is a plain
`SeriesConfig` value.

```rust
use renderer::{ScatterShape, SeriesConfig, color::Color, line::LineStylePreset};

let series = [
    SeriesConfig::line("sine", "x", "y").label("sin x").width(2.0),
    SeriesConfig::scatter("samples", "x", "noisy")
        .label("samples")
        .color(Color::from_rgb8(214, 39, 40))
        .marker(ScatterShape::Diamond, 7.0)
        .y_errors("sigma"),
    SeriesConfig::line("trend", "x", "fit").dash(LineStylePreset::Dash).fill_to(0.0),
];
```

The families: `line`, `scatter`, `scatter_line`, `stem`, `histogram`,
`heatmap`, `contours`, `hexbin`, `rgb_image`, `vectors`, `vectors_grid`,
`vectors_3d`, `streamlines`, `category_bars`, `box_plot`, `range_bars`,
`radar`, `ternary`, `parallel`, `mesh`, `energy_diagram` and
`feynman_diagram` (and `of_kind` for any `DataRenderType`). Matrices are
named with `MatrixRef::columns_are_x` or `columns_are_y` (and
`with_layout`). A family that needs its chart set up (a category axis,
polar, ternary or parallel axes) still validates against that chart.

One-line modifiers cover what most charts of a kind set:

- Any series: `label`, `color`, `width`, `dash`, `marker`, `z` (3D).
- Lines and markers: `with_line`, `no_markers`, `fill_to`, `fill_between`,
  `fill_color`, `interpolation` (steps), `with_stem`, `horizontal` (a
  histogram's bars or sideways stems).
- Error bars: `x_errors`, `y_errors`, `error_bars` (widths and cap size),
  `error_color`.
- Point maps: `color_by`, `size_by`.
- Histograms: `bar_border`, `bar_gap`. A horizontal histogram names its
  columns swapped (counts, then edges) and chains `horizontal()`.
- Range bars: `range_colors` (up/down candles), `whiskers` (wicks),
  `connector` (a waterfall), `bar_labels` (also category bars).
- Box plots: `violin`, `outliers`.
- Fields and meshes: `shading`, `opacity`, `filled`, `contour_levels`,
  `contour_labels`.
- Vectors: `vector_scale`, `arrow_head` (also streamline arrows), `pivot`.
- Ternary: `region`.

Everything else goes through a closure on the family's sub-style:
`style_line`, `style_markers` (including a size map's `scale`),
`style_stem` (a stem's own colour, width or dash), `style_bars` (category
and range bars), `style_vectors` (minimum length, colour grid),
`style_contours`, `style_surface` (3D look), and two that hand over every
field of their family as `&mut` parts (`renderer::helpers::RangeBarsParts`,
`BoxPlotParts`):

```rust
use renderer::SeriesConfig;
use renderer::color::Color;
use renderer::data_config::{BoxPoints, BoxPointsStyle, CategoryBarLabels, RangeProgress};

let tasks = SeriesConfig::range_bars("tasks", "day", "begin", "end", Color::from_rgb8(70, 110, 170))
    .bar_labels(CategoryBarLabels::Outside, owners)     // one string per bar
    .style_range_bars(|parts| {
        *parts.progress = Some(RangeProgress { column: "done".into(), remaining_opacity: 0.35 });
        *parts.slot_width = Some(1.0);                   // a numeric (day) position axis
    });
let commute = SeriesConfig::box_plot("commute", "pos", "record", Color::from_rgb8(60, 120, 110))
    .outliers("commute.outliers")
    .style_box_plot(|parts| {
        parts.style.notched = true;
        *parts.points = Some(BoxPoints {                 // a raincloud
            positions: "commute.cat".into(),
            values: "commute.value".into(),
            style: BoxPointsStyle::default(),
        });
    });
```

A modifier or closure that does not apply to a series' family leaves the
series as it is.

### Chart parts

With the `helpers` feature, the parts a host adds to a chart have
constructors too, and `Chart` takes them in a chain:

- `Color::with_alpha(a)`: the same colour at another alpha.
- `Annotation::reference_line(axis, value, color, width)`,
  `span(axis, from, to, fill)`, `text(anchor, text, color, size)` and
  `text_rich(anchor, rich_text)`, then `with_label`, `with_line_style`,
  `with_box` and `with_align`.
- `AxisOptions::overlay_y(title, color)`: the right-hand axis of an overlay
  layer, titled and coloured like its series.
- `AxisOptions::unit(scale, offset)`: the axis prints `scale * value +
  offset` ([#273](https://github.com/shim9610/figgy/issues/273)); see
  [Axis units](#axis-units).
- `Subplot::new(config, &series_ids)`, then `with_link_x(cell)` and
  `with_link_y(cell)`.
- `Chart::with_annotation`, `with_label_format` (both main axes),
  `with_x_unit` and `with_y_unit` (the bottom and left axes' units, see
  [Axis units](#axis-units)), `without_grid`, `with_colorbar_title`,
  `with_polar` and `with_subplot`.
- `Chart::with_label_clearance(mode)` sets `label_style.clearance` on every
  axis ([#272](https://github.com/shim9610/figgy/issues/272)).
  `LabelClearanceMode::Auto` (the default) drops a tick label where its box
  comes within a small visual gap of the label of the axis across at a
  corner or a slash of the axis' break mark, closer than the gap both
  across and along the screen. The gap is 2 px in total between two labels
  in the default 18 px tick label font; each box's half is read as that
  share of its own font, so it is `(a + b) / 18` px between an `a` px and
  a `b` px label and scales with the display or export. Boxes 1 px apart at
  18 px count as overlapping; labels the gap apart or more stay, and an
  axis keeps at least two. At a corner one axis yields (a y axis to the x axis, an x axis
  only to a categorical y axis, a 3D chart's z axis to the floor's labels)
  and its setting alone decides. `LabelClearanceMode::Keep` drops none. Set
  one axis' `label_style.clearance` for that axis alone.
- `Chart::placed(x, y, width, height)` puts the chart area at an offset and
  `with_area(..)` resizes a chart built from a document; `set_area(..)` moves
  an existing chart's area in place, as a window resize does;
  `with_overlay(y_axis, &series_ids)` adds an overlay layer;
  `with_legend_swatch(label, color)` adds a filled-square entry for a meaning
  that is not a series.

```rust
use renderer::{Annotation, AnnotationAxis, Chart, color::Color, config::AxisOptions};

let red = Color::from_rgb8(200, 60, 50);
let chart = Chart::sized(960, 600)
    .with_overlay(AxisOptions::overlay_y("Pressure (kPa)", red), &["pressure"])
    .with_annotation(Annotation::reference_line(AnnotationAxis::Y, 0.0, Color::BLACK, 1.0))
    .with_annotation(Annotation::span(AnnotationAxis::X, 2.0, 3.5, red.with_alpha(0.15)))
    .without_grid();
```

### Axis units

An axis can print its values in another unit while the data stays as the
host registered it ([#273](https://github.com/shim9610/figgy/issues/273)):
`AxisOptions::unit: Option<AxisUnit>` makes its tick labels read
`scale * value + offset`. Raw 16-bit ADC counts print as volts, kelvin as
degrees Celsius, seconds as milliseconds.

- The ticks are planned in the shown units, so the labels are round numbers
  after the conversion (`0, 1, 2 V`, not converted counts), and each tick
  sits at its data position `(shown - offset) / scale`. `major_spacing` is
  read in shown units.
- `min`, `max`, the data, `breaks`, `cross_at`, annotations and picks stay
  in data units.
- The label format applies to the shown value. A logarithmic axis takes
  only `offset = 0` and `scale > 0` and walks decades of the shown value.
  A probability axis, an axis with `Timestamp` labels and a categorical
  axis refuse a unit.
- `AxisOptions::unit(scale, offset)` (`helpers`) sets the unit and chooses
  the spacing for the shown range; for a main axis, set the field and
  then the range, as `set_y_range` and the `auto_fit_*` family choose the
  spacing of the shown range.
- `Chart::with_x_unit(scale, offset)` and `Chart::with_y_unit(scale,
  offset)` (`helpers`) do that for `bottom_x` and `left_y`: they set the
  unit and choose the spacing for the shown range, before or after the
  range. Like `with_label_format` they write those two axes alone, so
  `top_x`, `right_y` and an overlay's axis can print a second unit. A unit
  the axis refuses is stored as given, and `Config::validate_chart` names
  it; nothing panics.

```rust
let mut chart = Chart::sized(960, 600);
chart.set_x_range(250.0, 350.0);
chart.set_y_range(0.0, 32768.0);
let chart = chart
    .with_x_unit(1.0, -273.15)                         // kelvin printed as °C
    .with_y_unit(5.0 / 32768.0, 0.0);                  // counts printed as V
```

```rust
use renderer::{Chart, color::Color, config::{AxisOptions, AxisUnit}};

let mut chart = Chart::sized(960, 600)
    .with_y_title("Voltage (V)")
    .with_overlay(
        AxisOptions::overlay_y("Temperature (°C)", Color::from_rgb8(176, 64, 32))
            .unit(1.0, -273.15),                       // kelvin printed as °C
        &["temperature"],
    );
chart.config_mut().left_y.unit = Some(AxisUnit::new(5.0 / 32768.0, 0.0)); // counts as V
chart.set_y_range(0.0, 32768.0);                       // ticks at 0, 1, 2, 3, 4, 5 V
```

### Math in labels

The convenience functions that take text as a `&str` read it the way
matplotlib reads a label: `Chart::with_title`, `with_x_title`,
`with_y_title`, `with_legend_entry`, `with_colorbar_title`,
`with_legend_swatch`, `SeriesConfig::label`, `Annotation::text`,
`AxisOptions::overlay_y`, `RadialChart::with_title`, and the browser text
setters ([WASM.md](WASM.md#math-in-labels)).

- `$…$` and `\(…\)` spans are TeX in text style, `$$…$$` spans in display
  style. Text outside the spans is plain.
- `\$` is a dollar sign: `r"Price (\$)"`.
- Text whose TeX does not parse is shown as written, with each `\$` as
  `$`; the wrapper never fails.

The same reading is public as `text::rich_segments_from_label`
(`RichText::from_label` for a whole `RichText`). The document types stay
explicit: `RichText::plain` is plain
text, and `RichText::from_tex` returns a `TexError` naming the bytes that do
not parse.

```rust
use renderer::{Chart, SeriesConfig, color::Color, text::RichText};

let fit = SeriesConfig::line("fit", "t", "fit").label(r"$\hat y \pm 1.96\sigma$");
let chart = Chart::sized(960, 600)
    .with_x_title("Time (s)")
    .with_y_title(r"$\sin(2\pi f t)$")
    .with_legend_from(&[fit]);

// Low level: the document's own text, explicit and checked.
let title = RichText::from_tex(r"$E = mc^2$", Color::BLACK, 18.0, "")?;
```

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
- **Triangle meshes**: `DataRenderType::TriMesh` draws a colour fill and
  contour lines over triangles the host gives, and a surface in the box of a
  3D chart. Upload the vertex coordinates and values as columns and the
  triangles with `Renderer::upsert_triangles(id, &TriangleSource { indices })`
  (three vertex rows per triangle, from a `Column<u32>`); the renderer never
  triangulates. [Guide](TRIMESH.md) and the `TriMesh` / `TriSurface`
  playground scenes.
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

### Monotonic-x lines

For fast, large, evenly spaced data (`x = start + i × interval`), such as a
data acquisition stream, `SeriesConfig::monotonic_line(id, group, channel)`
draws one channel of a group as a line through its **M4 envelope**: per
pixel column it keeps the first, lowest, highest and last samples, so the
cost of a frame depends on the plot width and the channel count, not on the
number of samples. The samples stay with the host: every call lends them through a
`MonotonicSource` (`MonotonicSlices` for one slice per channel,
`MonotonicInterleaved` for one sample-major buffer with a stride; `f64`,
`f32`, `i32`, `i16`, `u16` or `u8`), and the source is not retained after
the call.

```rust,ignore
use renderer::renderer::{MonotonicGroupRef, MonotonicRange};
use renderer::layout::Side;
use renderer::{MonotonicAppend, MonotonicSlices, SeriesConfig};

let series = vec![
    SeriesConfig::monotonic_line("ch0", "daq", 0).label("channel 0"),
    SeriesConfig::monotonic_line("ch1", "daq", 1).label("channel 1"),
];
// A chart with monotonic series is a monotonic chart, reached only through
// its handle: no general chart call takes it.
let id = renderer.register_monotonic_chart(chart.config().clone(), series)?;
// The source carries the samples, start and interval. The full call reduces
// them in full; `Autoscale` fits the x range to the samples and the y range
// to what this reduction keeps, `X { min, max }` sets the x range.
let channels = [ch0.as_slice(), ch1.as_slice()];
let source = MonotonicSlices::new(0.0, 1e-3, &channels);
let mut daq = renderer.monotonic_chart(id)?;
daq.set_monotonic("daq", &source, MonotonicRange::Autoscale)?;
// Then only the new samples (an append continues the full call's grid);
// `sliding` keeps the window's length and moves the x range by their span.
let fresh = [new0.as_slice(), new1.as_slice()];
daq.append_monotonic("daq", &MonotonicSlices::new(0.0, 1e-3, &fresh), MonotonicAppend { sliding: true })?;
// A zoom is a full call at the new range.
daq.set_monotonic("daq", &source, MonotonicRange::X { min: 0.0, max: 2.0 })?;
// A resize (the chart area, or the plot a plot-area resize proposes) and a
// layout fit take the groups' sources too.
let groups = [MonotonicGroupRef::new("daq", &source)];
daq.resize(new_chart_area, None, Some((width, height)), &groups)?;
daq.fit_layout(&groups)?;
// Everything else edits the document through its editor, with no source.
daq.edit_config(|config| {
    config.chart_title().visible = true;
    config.axis(Side::Left).title().visible = true;
})?;
```

Only these four calls move the x-to-pixel mapping (the x range, the plot's
width in device pixels, the x direction), and each takes the source. The
general chart calls take a `ChartId`, which a monotonic chart never hands
out, so they cannot reach it; `register_chart` and the series calls refuse a
monotonic line series (`RuleKind::MonotonicGeneralChart`). The document
editor (`MonotonicConfig`) edits the legend, titles, axis titles, ticks,
tick labels and their format, colours, annotations, the selection and
decoration drags, and has no setter for the x range, x scale or plot area;
an edit that would move them is refused (`RuleKind::MonotonicMappingEdit`).
A line width is a plain style edit (`set_series`). Strokes up to about 120
device pixels wide draw their edges exactly; a wider one may miss ink at the
plot edges. Frames and picks never need the source.
`renderer.monotonic_needs_source(id)` lists the series whose group was never
supplied. `prepare_monotonic` draws `MonotonicChartDrawItem`s, and an export
takes `ExportSource::Monotonic { chart: id, monotonic: &groups }` (`&[]` at
the screen's scale).
Picks report the logical sample index of a kept point. Log, probability and
broken x axes are refused. See [MONOTONIC.md](https://github.com/shim9610/figgy/blob/master/crates/model/MONOTONIC.md).

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

A polar plot is a Cartesian chart with `config.set_polar(Some(PolarOptions { .. }))`:
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

### Headless renderer and saving by format

With the `helpers` feature, a renderer without a window takes one call, and
one call exports and writes a file in the format its name gives. The steps
stay explicit: create, upload, set the document, save.

```rust
use renderer::{Chart, ExportOptions, ExportSource, Renderer, SeriesConfig};

let mut renderer = Renderer::headless_blocking()?;        // instance, adapter, device, pipelines
renderer.add_column("x", &xs)?;                          // a Vec<f64> or a slice, no wrapper
renderer.add_column("y", &ys)?;
let series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
let mut chart = Chart::sized(960, 600).with_title("A sine wave").with_legend_from(&series);
chart.fit_layout();                                       // margins fitted to the labels
let source = ExportSource::Chart { chart: &chart, series: &series };
renderer.save_blocking(source, ExportOptions::document(2.0), "sine.png")?;  // or .svg / .pdf
```

- `Renderer::headless_blocking()` (native) and `Renderer::headless(options).await`
  (also in the browser) honour `WGPU_BACKEND`; `HeadlessOptions` sets the
  initial column pool (16 MiB by default) and the power preference
  (`headless_blocking_with`).
- `ExportFormat` is `Png`, `Svg` or `Pdf`; `ExportFormat::from_path` reads it
  from the extension in any letter case. `export_bytes` (async) and
  `export_bytes_blocking` return the bytes; `save_blocking` picks the format
  from the extension and `save_as_blocking` takes it explicitly. Each routes
  to the export below, so the file is exactly what that export returns.
- A save fails with `SaveError`: the export's own error, an extension that
  names no format, or the file write.
- `RadialRenderer` has the same `export_bytes`, `export_bytes_blocking` and
  `save_blocking`, with the chart and its size.
- `Chart::sized(width, height)` starts from the documented default look with
  that chart area; `with_legend_from(&series)` adds one legend entry per
  labelled series with the series' own symbol; `fit_layout()` fits the axis
  bands and margins to their labels (`layout::auto_layout`).
- `RadialRenderer::headless_blocking()` makes a radial renderer the same way,
  `RadialChart::with_title(text)` gives the chart a title in its own font and
  label colour, and `RasterImage::save_png(path)` writes an `export_rgba`
  image as a PNG file, for a host that inspects the pixels before saving.
- The chart's own placement, overlays, annotations and subplots:
  [Chart parts](#chart-parts).

### Themes

A theme (`renderer::theme::Theme`, `helpers` feature) writes a whole look into
the document once. It is a value, not a name the document stores: after it is
applied, the document holds plain values and stays the single source of truth.

```rust
use renderer::theme::{SubplotScope, Theme};

let mut series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
let mut chart = Chart::sized(960, 600).with_title("A sine wave").with_legend_from(&series);
chart.apply_theme(&Theme::dark(), &mut series, SubplotScope::All);  // refits the layout too
```

- Built in: `Theme::light()` (the documented default look), `dark()`,
  `paper()` (open frame, no grid, grayscale-safe), `dashboard()` (no axis
  lines or ticks, horizontal grid, large text) and `sketch()` (hand-drawn
  strokes). `Theme::named("dark")` looks one up by name. Every field is public,
  so a host edits a copy before applying it.
- A theme writes the chrome (`ChromeTheme`): the chart and plot fills, the
  draw style, the axis frame (`AxisPreset`), axis, tick-label, title and legend
  colours, fonts and sizes, the grid, the legend box, the colourbar outline and
  the selection highlight. It writes each series' family part (`SeriesTheme`):
  colours through its `ColorCycle`, stroke widths, marker sizes and the
  family's materials. Energy-level and Feynman charts keep a frame of their
  own (`Grotrian`, `Hidden`).
- It keeps axis ranges, scales, tick spacing, label formats, categories,
  axis breaks, the chart area, every offset, all text content, the annotation
  layer, the overlay and subplot structure, series ids, columns and labels,
  and the selection itself.
- Overlays take the theme with their chart. `SubplotScope` chooses the
  subplots: `None`, `All` or `Only(&[indices])`; a series drawn in a subplot
  outside the scope keeps its look.
- `Chart::apply_theme` refits the layout to the new text sizes
  (`fit_layout`). `Chart::with_theme(&theme)` applies the chrome alone and
  fits; style the series with `theme.style_series(&mut series, index)`.
  `theme.apply(&mut config, &mut series, scope)` works on a bare `Config` and
  leaves the layout to the host. `theme.apply_radial(&mut radial_chart)`
  themes a radial chart.
- Applying the same theme twice changes nothing more.

### PNG export (memory only — saving is the caller's job)

```rust
let bytes = renderer
    .export_png(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::panel(scale))
    .await?;
std::fs::write("/tmp/out.png", &bytes)?;          // or clipboard / network / wherever.

// If you only need RGBA:
let img = renderer
    .export_rgba(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::panel(scale))
    .await?;
// img.width, img.height, img.rgba (straight alpha, length = w * h * 4)
```

Native hosts can also call `export_png_blocking` and `export_rgba_blocking`
with the same arguments.
In the snippet, `series_configs` holds the `SeriesConfig` values to draw (for the
setup example above, `std::slice::from_ref(&cfg)`) and `scale` is the export
scale, such as `2.0`. The image is the chart area times `scale` on a fully
transparent background. For a background colour, pass
`ExportOptions::panel(scale).with_clear(Color::WHITE)`.
The panel export (`ExportOptions::panel`, `ExportScope::Panel`) draws the base
chart area only. For a chart with `overlays` or `subplots`,
`ExportOptions::document(scale)` draws every layer and subplot into one image
covering all chart areas.
`scale` is at least `renderer::MIN_EXPORT_SCALE` (0.25; NaN and values at or below zero become it) and has no upper bound: the scaled image must fit the device's texture limit (`max_texture_dimension_2d`, commonly 8192 or 16384 px per side), and a larger export fails with `FiggyError::GpuResourceLimit` before anything is allocated.
Convert from standard 96 DPI via `renderer::dpi_to_scale(dpi)`.

When scaling, every pixel-based dimension (font, line, margin, grid, legend)
scales by the same factor, so the layout matches the on-screen chart at a
higher pixel density (or a lower one below 1.0).

### SVG and PDF export

```rust
let svg: String = renderer
    .export_svg(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::document(scale))
    .await?;
let pdf: Vec<u8> = renderer
    .export_pdf(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::document(scale))
    .await?;
std::fs::write("/tmp/out.svg", svg)?;
std::fs::write("/tmp/out.pdf", pdf)?;
```

The two documents hold the same picture as `export_png`
with the same arguments ([#48](https://github.com/shim9610/figgy/issues/48)).
The decorations are vector content, written from the placement the PNG
draws with: axes, ticks, tick labels, grid, polar rings and spokes, axis and
chart titles, legend, the colourbar's border, axis and labels, and the
annotation layer. Text is written as glyph outlines, so no font is embedded
and the text cannot be selected or searched. The data layer (fills, lines,
markers, bars, fields, contours, vectors, picked highlights and value
labels) is one image at `scale ×` resolution, with the PNG export's own
pixels: one image for a chart and its overlays, one more for each subplot. A
series defined by its configuration alone, such as an energy-level diagram,
is vector content too, placed as in the PNG
([#234](https://github.com/shim9610/figgy/issues/234)).
It sits between the other data under it and the value labels over it, so a
chart or subplot with one is written as an image of the data under it, the
series, then an image of the labels; an image with no other series in it is
left out. The colourbar ramp and the
Milkyway backdrop are images too; the Milkyway glow is not drawn. Selection
boxes are never drawn, as in the PNG export.

The SVG's `viewBox` is in physical pixels and its `width` / `height`, like
the PDF page, are the logical size in points (1 px = 0.75 pt), so a 2x export
has the same page size with twice the image resolution. `scale` is clamped
as for the PNG export, and a background colour set with
`ExportOptions::with_clear` is a rectangle under everything. Both exports
take the whole document: `ExportOptions::panel` is refused with
`FiggyError::InvalidConfig`. Every export is async, and a native host can
also call its `_blocking` twin (see
[GPU results](#gpu-results-awaited-blocking-or-checked)).
`WindowedRenderer` has the same methods. A series with a streamed
source is rejected with `FiggyError::ColumnNotResident`, as by the PNG
export.

The radial chart exports with `RadialRenderer::export_svg(&chart, size,
options)` and `export_pdf` (blocking: `export_svg_blocking` /
`export_pdf_blocking`): the
slices and background are one image, the title, labels, leader lines, split
connectors and annotations are vector content. Every inside label is drawn
above every slice at its place in the PNG, and no selection box is drawn.

---

---

## 1. 사용법

### 툴체인과 빌드

[`rust-toolchain.toml`](https://github.com/shim9610/figgy/blob/master/rust-toolchain.toml)에 개발 환경을 Rust 1.99.0으로 고정하고 wasm32 빌드 대상, rustfmt, clippy를 지정했다. rustup이 설치돼 있으면 저장소 안에서 명령을 실행할 때 자동으로 적용된다. 패키지 manifest에도 지원 하한을 Rust 1.99로 명시했다. 그보다 낮은 버전은 검증하지 않았다.
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
`Cargo.toml` 기준 경로를 지정한다. 네이티브 프로그램에는 비동기 실행기가 필요 없다. GPU를
기다리는 호출에는 모두 `_blocking` 형태가 있다([GPU 결과](#gpu-결과-비동기-동기-확인)).
[examples/line.rs](examples/line.rs)도 이 형태를 쓴다.

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
`ChartStyle`도 보관한다. 축·격자·라벨·범례·패널 크기를 `Chart`로 바꿨다면 그대로 prepare하면
된다. `prepare`는 뷰를 그린 뒤로 차트가 바뀌었으면 글자·축 래스터를 다시 그리고,
`chart.select(..)`로 지정한 요소를 선택 상태로 그린다. `refresh_axis`(또는
`refresh_axis_with_selection`)는 래스터를 직접 다시 그리려는 호스트가 쓴다. 시리즈의
모양·매핑 스타일·식별자를 바꿨다면 `create_style_for_series`로 `ChartStyle`을 다시 만든다.
특정 표시 배율로 만든 스타일은 같은 배율로 `create_style_for_series_scaled`를 호출한다.
`prepare`는 스타일을 다시 만들지 않는다. `Renderer::register_chart`로 등록한 차트는 자체
스타일 캐시를 두고 `set_chart_config` / `set_chart_series`로 갱신하며, `prepare_registered`가
저장된 차트와 그 선택 상태로 호출자의 `ChartView`를 다시 그린다.
[편집 표와 컴파일되는 예제](API.md#cartesian-edits-update-the-derived-inputs-before-preparing)를
참고한다.

래스터를 다시 그리면 축 래스터가 설정대로 그리지 못한 눈금 설정도 알려 준다. 뷰를 다시
그린 prepare 뒤에 `view.render_warnings()`를 읽는다. 각 `RenderWarning`에는 축, 설정한 값,
실제로 그린 값이 들어 있다. 예를 들어 간격이 1000개를 넘어야 하는 `major_spacing`은 자동
간격으로 그린다. 목록은 뷰를 래스터화할 때만 다시 만들므로 프레임마다 비용이 들지 않는다. 바뀐 경고만 보여 주려면 이전 목록과 비교한다.
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
let mut renderer = Renderer::for_window_blocking(
    Arc::clone(&window),
    (size.width, size.height),
    16 * 1024 * 1024,   // GPU column pool 초기 16 MiB, 필요하면 늘어난다
).unwrap();

// renderer.add_column은 `&dyn ColumnSource`를 받는다. f64, f32, u32, Option<f64>의
// `Vec`이나 슬라이스는 그대로 넘긴다. ndarray, polars Series, mmap 같은 다른 데이터는
// 그 타입에 트레이트를 구현해 넘긴다(아래 `ColumnSource` 절 참고).
let xs: Vec<f64> = (0..1024).map(|i| i as f64 * 0.01).collect();
let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
renderer.add_column("x", &xs).unwrap();
renderer.add_column("y", &ys).unwrap();

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
    x_column: "x".into(), y_column: "y".into(), z_column: None,
    render_type: DataRenderType::Xy(renderer::data_config::XySeries { scatter: None, line: Some(DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::from_rgb8(20, 110, 230),
            line_width: 2.0,
            fill: None,
            fill_color: None,
            interpolation: renderer::data_config::LineInterpolation::Linear,
        }), errors: None, stem: None }),
};
let style = renderer.create_style_for_series(&cfg).unwrap();   // budget-checked ChartStyle
let mut view = renderer.create_chart_view(&chart, chart.config().chart_area.0).unwrap();

// frame loop:
let series = [Series { config: &cfg, style: &style }];
let mut items = [ChartDrawItem {
    view: &mut view,
    chart: &chart,
    series: &series,
}];
renderer.draw(Color::WHITE, &mut items).unwrap();   // surface frame 획득 → prepare(오래된 래스터를 다시 그림) → encoder → pass → paint_prepared → submit → present
```

<a id="columnsource--데이터-어댑터-trait"></a>
<a id="columnsource--데이터-어댑터-트레이트"></a>

### `ColumnSource`: 데이터 어댑터 트레이트

`Renderer::add_column`은 `&dyn ColumnSource`를 받는다. 사용 중인 데이터 타입에 이 트레이트를 구현하면 된다. 값은 호출 중에 올라가므로 호출이 끝나면 호스트는 컨테이너를 버리거나 고쳐도 된다.

데이터 소스는 값만 기록한다. 소스가 값을 쓰는 `ColumnPairWriter`가 같은 순회에서 컬럼 통계, 즉 유한한 최솟값과 최댓값, 0보다 큰 가장 작은 유한값을 기록한다. `writer.write_value(index, v)`로 f64를 쓰면 `(hi, lo)`로 나눠 저장하고 통계에는 `v` 자체를 반영한다. `writer.write_pair(index, hi, lo)`는 `hi as f64 + lo as f64`를 반영한다. GPU가 0으로 읽는 값(f32 범위보다 작은 값)은 0으로 기록하고, 유한하지 않은 값은 통계에서 뺀다. 자동 맞춤이 이 통계를 읽으며, 데이터에 0이나 음수가 있으면 로그축 자동 맞춤은 0보다 큰 가장 작은 값을 하한으로 쓴다. 스트리밍 소스도 청크마다 같은 방식으로 통계를 낸다.

큰 Unix 시각이나 절대 좌표처럼 f32로는 작은 차이가 사라지는 값은 `Renderer::add_hilo_column`에 `&dyn HiLoColumnSource`를 넘겨 등록한다. 기본 제공 구현체 중 `f64`, `f32`, `Option<f64>`를 담는 것은 이 트레이트도 구현한다. 브라우저에서 큰 Unix 시각을 시간축에 쓸 때는 처음 등록할 때 `register_column_f64`, 교체할 때 `update_register_column_f64`에 `Float64Array`를 넘긴다.

`add_column` / `add_hilo_column`은 새 ID를 등록할 때 사용한다. 기존 ID의 데이터를 원자적으로 교체하려면 `upsert_column` / `upsert_hilo_column`을 사용한다. 교체에 실패하면 오류를 돌려주고, 기존 컬럼과 그 컬럼을 쓰는 차트, 피킹은 그대로 남는다.

호스트가 별도의 파생 상태를 관리한다면 `begin_upsert_*`가 반환한 가드의 `pool()`로 임시 풀을 읽어 그 상태를 미리 준비한 뒤 가드의 `commit`을 호출한다. `commit`은 실패하지 않는다. 가드를 커밋하지 않고 버리면 기존 컬럼이 그대로 남는다. 어느 쪽이든 피킹 갱신은 렌더러가 맡는다.

```rust
pub trait ColumnSource {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool { self.len() == 0 }  // 디폴트 제공

    /// 모든 값을 `(value as f32, 0)` 쌍(또는 `(hi, lo)` 분할)으로 기록한다.
    /// `0..len()`의 각 인덱스를 한 번씩 쓴다. null은 `f32::NAN`으로 기록한다.
    fn write_pairs(&self, writer: &mut ColumnPairWriter<'_>);

    /// 스트리밍 청크 하나: `start`부터 `writer.len()`개의 값을 쓴다.
    /// 스트리밍하지 않는 소스는 `Unsupported`를 반환하는 기본 구현을 쓴다.
    fn write_pair_range(
        &self,
        start: u64,
        writer: &mut ColumnPairWriter<'_>,
    ) -> Result<(), ColumnRangeWriteError>;
}
```

**기본 제공 구현체**: `T`가 `f64`, `f32`, `u32`, `Option<f64>`일 때의 `Column<T>`, `Vec<T>`, `[T]`, `&[T]` (`None` → NaN). 배열은 구현체가 아니므로 `&arr.as_slice()`로 넘긴다.

`write_pairs`에는 기본 구현이 없으므로 모든 소스가 직접 구현한다. 컬럼 통계는 언제나 소스가 쓴 값에서 나온다. `HiLoColumnSource`도 같은 이름의 메서드로 `(hi, lo)` 쌍을 기록한다.

**사용자 정의 구현 예: 시계열·DataFrame·mmap·FFI 데이터**

```rust
struct MyTimeSeries {
    samples: Vec<f64>,    // 또는 Arc<[f64]>, ndarray::ArrayView, polars::Series, ...
}

impl renderer::ColumnSource for MyTimeSeries {
    fn len(&self) -> usize { self.samples.len() }
    fn write_pairs(&self, writer: &mut renderer::ColumnPairWriter<'_>) {
        debug_assert_eq!(writer.len(), self.samples.len());
        for (index, &sample) in self.samples.iter().enumerate() {
            writer.write_pair(index, sample as f32, 0.0);
        }
    }
}

renderer.add_column("temperature", &my_series)?;   // my_series: MyTimeSeries
```

네이티브 `f32` 컨테이너도 같다. 값을 순회하며 `writer.write_pair(index, value, 0.0)`를 호출하면 된다. `ColumnPairWriter`는 인덱스로 값 쌍을 하나씩 받을 뿐 매핑된 바이트를 노출하지 않으므로, 소스가 wgpu 타입을 다룰 일은 없다.

<a id="네이티브-example--사인--rc--cross-section"></a>

### 네이티브 예제: 사인 곡선, RC 회로, 단면 그래프

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
        // 래스터와 transform은 따로 처리하지 않는다. prepare가 오래된 래스터를 다시 그리고
        // transform도 직접 갱신한다. 패널은 뷰를 쓰기 가능하게, 차트와 시리즈는 읽기 전용으로
        // 빌려준다(서로 다른 필드). 그래서 이 `&mut` 호출에 `state.renderer`를 그대로 쓸 수 있다.
        let prepared = state.renderer.prepare(&mut items).unwrap();
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

`WindowedRenderer::draw*`나 패널 내보내기(`ExportScope::Panel`)처럼 렌더러가 직접 제출하는 경로에서는 이 처리를 내부에서 수행한다. 외부 렌더 패스 기록과 이 경로를 함께 쓴다면 미리 기록해 둔 명령 버퍼를 먼저 모두 제출해야 한다. 렌더러는 wgpu 명령 버퍼의 내부 참조를 볼 수 없으므로 호스트가 아직 제출하지 않은 자원만 따로 골라 보호할 수 없다.

`paint_prepared`는 같은 토큰으로 여러 패스에 호출할 수 있다. `PreparedFrame`이 그리기에 필요한 입력을 보관하므로 paint 단계에서 `items`를 다시 만들거나 전달하지 않는다. 준비 후 관련 렌더러 자원이 바뀌면 아무 명령도 기록하지 않고 `FiggyError::StalePreparedFrame`을 반환한다. 다음 프레임에서 다시 `prepare`하면 된다.

토큰이 기록한 등고선 라벨(자동 배치와 직접 지정한 앵커 모두), 계산한 경로 길이, 별 위치는 토큰을 해제한 뒤에도 이후의 `prepare`가 덮어쓰지 않는다. 따라서 그 토큰으로 기록한 명령 버퍼는 제출할 때까지 유효하다.

토큰으로 기록한 명령 버퍼는 그 토큰의 `ChartView`를 다시 바꾸기 전에 제출해야 한다. `refresh_axis`, `update_transform`, 같은 뷰로 실행하는 다음 `prepare`가 여기에 해당한다. 렌더러는 기록하기 전에 오래된 입력을 거부하지만, 호스트에 넘어간 명령 버퍼는 검사하거나 순서를 정할 수 없다.

winit 루프나 WASM 래퍼처럼 프레임 동안 렌더러를 단독으로 사용하는 호스트는 두 단계를 연속 실행하는 `Renderer::paint(&mut self, …)`를 사용하면 된다. 전체 예제는 [egui_embed.rs](examples/egui_embed.rs)를 참고한다. 이 예제는 `end_gpu_frame`을 호출하지 않는다. `queue.submit` 뒤에 한 번 호출하는 호스트는 [tools/playground/src/main.rs](https://github.com/shim9610/figgy/blob/master/tools/playground/src/main.rs)에서 볼 수 있다.

### GPU 결과: 비동기, 동기, 확인

픽, extent, 원형 차트 픽, 내보내기와 `wait_submitted_work`는 GPU 결과를 읽어 온다.
세 가지 방법으로 기다릴 수 있고 결과는 같다. 기다리는 방법만 다르다.

| 대기 | 호출 | 기다리는 곳 |
|---|---|---|
| 비동기 | `ticket.resolve().await`, `renderer.export_png(..).await`, `wait_submitted_work().await` | 네이티브: 장치 묶음의 poll 스레드가 장치를 처리하고, future는 작업이 끝날 때까지 `Pending`을 돌려준다. 브라우저: 브라우저가 장치를 처리한다. |
| 동기(네이티브) | `ticket.resolve_blocking()`, `export_png_blocking(..)`, `wait_submitted_work_blocking()` | 호출한 스레드가 작업이 끝날 때까지 장치를 처리한다. |
| 확인 | `ticket.try_resolve()` | 기다리지 않는다. 작업이 끝났으면 `TryResolve::Ready(결과)`, 아니면 `TryResolve::Pending(티켓)`을 돌려주므로 다음 프레임에 다시 확인한다. |

네이티브에서 `RendererDevice::new`는 장치 묶음마다 poll 스레드를 하나 띄운다. 묶음의
복제본, 묶음으로 만든 렌더러, 기다리는 티켓이 이 스레드를 함께 소유한다. 기다리는 작업이
없으면 스레드는 잠들고, 소유자가 모두 사라지면 끝난다. 동기 함수만 쓰는 호스트는
`RendererDevice::blocking`으로 스레드 없는 묶음을 만든다. 이 묶음에서 비동기 대기를
부르면 끝나지 않는 대신 `FiggyError::AsyncWaitUnavailable`을 돌려준다. 스트리밍
시리즈의 티켓은 `resolve`로만 읽는다(`GpuPickError::Streamed`).

즉시 모드 UI의 호버 픽에는 `try_resolve`가 맞는다. 포인터가 움직일 때 픽을 제출하고,
티켓을 패널 상태에 두었다가 매 프레임 확인한다. 막히는 스레드도 없고 티켓에 잠금도
필요 없다.

```rust
// 개요: `pending: Option<GpuPickTicket>`은 패널 상태에 둔다.
if let Some(ticket) = pending.take() {
    match ticket.try_resolve()? {
        renderer::TryResolve::Ready(hit) => hover = hit,
        renderer::TryResolve::Pending(ticket) => pending = Some(ticket),
    }
}
```

### iced 통합 상태

[보관 중인 iced 통합 예제](https://github.com/shim9610/figgy/blob/master/crates/renderer/unsupported/iced_embed_wgpu27.rs)에서 `prepare` / `paint_prepared`의 소유권 패턴을 확인할 수 있다. 다만 iced 0.14는 wgpu 27을 사용하므로 현재 빌드 대상에서는 제외한다. 서로 다른 wgpu 주 버전의 장치·큐·렌더 패스 타입은 공유할 수 없다.

### 시리즈 생성 함수

`helpers` 기능(기본으로 켜져 있다)을 쓰면 차트 종류마다 `SeriesConfig`에 생성 함수가 있다. 생성
함수는 컬럼을 지정하고 호스트용 기본 스타일로 그린다. 이어 붙이는 수정 메서드로 공통 필드를 바꾸고,
그 밖의 필드는 돌려받은 시리즈에서 직접 고친다. 결과는 평범한 `SeriesConfig` 값이다.

```rust
use renderer::{ScatterShape, SeriesConfig, color::Color, line::LineStylePreset};

let series = [
    SeriesConfig::line("sine", "x", "y").label("sin x").width(2.0),
    SeriesConfig::scatter("samples", "x", "noisy")
        .label("samples")
        .color(Color::from_rgb8(214, 39, 40))
        .marker(ScatterShape::Diamond, 7.0)
        .y_errors("sigma"),
    SeriesConfig::line("trend", "x", "fit").dash(LineStylePreset::Dash).fill_to(0.0),
];
```

생성 함수: `line`, `scatter`, `scatter_line`, `stem`, `histogram`, `heatmap`, `contours`, `hexbin`,
`rgb_image`, `vectors`, `vectors_grid`, `vectors_3d`, `streamlines`, `category_bars`, `box_plot`,
`range_bars`, `radar`, `ternary`, `parallel`, `mesh`, `energy_diagram`, `feynman_diagram`. 그 밖의
`DataRenderType`은 `of_kind`로 만든다. 행렬은 `MatrixRef::columns_are_x`나 `columns_are_y`로
지정한다(`with_layout`도 있다). 차트 설정이 필요한 종류(범주 축, 극좌표, 삼각 좌표, 평행 좌표)는 그
차트 설정에 맞춰 검증된다.

한 줄짜리 수정 메서드는 그 종류의 차트에서 대개 바꾸는 값을 다룬다.

- 모든 시리즈: `label`, `color`, `width`, `dash`, `marker`, `z`(3D).
- 선과 마커: `with_line`, `no_markers`, `fill_to`, `fill_between`, `fill_color`,
  `interpolation`(계단), `with_stem`, `horizontal`(히스토그램 막대를 가로로, 줄기를 옆으로).
- 오차 막대: `x_errors`, `y_errors`, `error_bars`(선 두께와 끝선 길이), `error_color`.
- 점 매핑: `color_by`, `size_by`.
- 히스토그램: `bar_border`, `bar_gap`. 가로 히스토그램은 컬럼을 바꿔(개수, 구간 경계 순서로) 넘기고
  `horizontal()`을 붙인다.
- 범위 막대: `range_colors`(캔들의 상승·하락 색), `whiskers`(꼬리), `connector`(폭포 차트),
  `bar_labels`(범주형 막대에도 쓴다).
- 박스플롯: `violin`, `outliers`.
- 필드와 메시: `shading`, `opacity`, `filled`, `contour_levels`, `contour_labels`.
- 벡터: `vector_scale`, `arrow_head`(유선의 화살표에도 쓴다), `pivot`.
- 삼각 좌표: `region`.

나머지 필드는 하위 스타일을 받는 클로저로 바꾼다. `style_line`, `style_markers`(크기 매핑의
`scale` 포함), `style_stem`(줄기만의 색, 두께, 점선), `style_bars`(범주형·범위 막대),
`style_vectors`(최소 길이, 색 격자), `style_contours`, `style_surface`(3D 모양)가 있다. 범위 막대와
박스플롯은 그 종류의 필드를 모두 `&mut`로 묶어 넘기는 클로저를 쓴다(`renderer::helpers::RangeBarsParts`,
`BoxPlotParts`).

```rust
use renderer::SeriesConfig;
use renderer::color::Color;
use renderer::data_config::{BoxPoints, BoxPointsStyle, CategoryBarLabels, RangeProgress};

let tasks = SeriesConfig::range_bars("tasks", "day", "begin", "end", Color::from_rgb8(70, 110, 170))
    .bar_labels(CategoryBarLabels::Outside, owners)     // 막대마다 문자열 하나
    .style_range_bars(|parts| {
        *parts.progress = Some(RangeProgress { column: "done".into(), remaining_opacity: 0.35 });
        *parts.slot_width = Some(1.0);                   // 숫자(날짜) 위치 축
    });
let commute = SeriesConfig::box_plot("commute", "pos", "record", Color::from_rgb8(60, 120, 110))
    .outliers("commute.outliers")
    .style_box_plot(|parts| {
        parts.style.notched = true;
        *parts.points = Some(BoxPoints {                 // 레인클라우드
            positions: "commute.cat".into(),
            values: "commute.value".into(),
            style: BoxPointsStyle::default(),
        });
    });
```

시리즈 종류에 맞지 않는 수정 메서드나 클로저는 시리즈를 바꾸지 않는다.

### 차트 부품

`helpers` 기능을 쓰면 호스트가 차트에 더하는 부품에도 생성 함수가 있고, `Chart`에 이어 붙여 넣을 수
있다.

- `Color::with_alpha(a)`: 알파만 바꾼 같은 색.
- `Annotation::reference_line(axis, value, color, width)`, `span(axis, from, to, fill)`,
  `text(anchor, text, color, size)`, `text_rich(anchor, rich_text)`. 여기에 `with_label`,
  `with_line_style`, `with_box`, `with_align`을 붙인다.
- `AxisOptions::overlay_y(title, color)`: 오버레이 레이어의 오른쪽 축. 제목과 색을 그 축이 맡은
  시리즈에 맞춘다.
- `AxisOptions::unit(scale, offset)`: 축이 `scale * value + offset`을 찍는다
  ([#273](https://github.com/shim9610/figgy/issues/273)). 아래 [축 단위](#축-단위)를 참고한다.
- `Subplot::new(config, &series_ids)`. 축을 이으려면 `with_link_x(cell)`, `with_link_y(cell)`를 붙인다.
- `Chart::with_annotation`, `with_label_format`(두 주 축), `with_x_unit`과 `with_y_unit`(아래쪽·왼쪽
  축의 단위, [축 단위](#축-단위) 참고), `without_grid`, `with_colorbar_title`, `with_polar`,
  `with_subplot`.
- `Chart::with_label_clearance(mode)`는 모든 축의 `label_style.clearance`를 정한다
  ([#272](https://github.com/shim9610/figgy/issues/272)). `LabelClearanceMode::Auto`(기본값)는
  모서리에서 눈금 라벨 상자가 건너편 축의 라벨이나 그 축의 끊김 표시 사선에 가로·세로 모두 작은
  시각적 간격보다 가까워지면 그 라벨을 뺀다. 간격은 기본 눈금 라벨 글꼴(18 px)인 두 라벨 사이에서
  합계 2 px이다. 각 상자가 자기 글꼴 크기에 비례해 절반씩 맡으므로 `a` px 라벨과 `b` px 라벨
  사이는 `(a + b) / 18` px이고, 화면·내보내기 배율에 따라 커진다. 18 px에서 1 px 떨어진 상자는
  겹친 것으로 보고, 간격 이상 떨어진 라벨은 그대로 둔다. 축에는 라벨을 두 개 이상 남긴다.
  모서리에서는 한 축만 비켜 주며(y축은
  x축에, x축은 범주형 y축에만, 3D 차트의 z축은 바닥 축 라벨에) 그 축의 설정만 따른다.
  `LabelClearanceMode::Keep`은 라벨을 하나도 빼지 않는다. 한 축만 바꾸려면 그 축의
  `label_style.clearance`를 쓴다.
- `Chart::placed(x, y, width, height)`는 차트 영역을 원하는 위치에 두고, `with_area(..)`는 문서로
  만든 차트의 크기를 바꾸며, `set_area(..)`는 창 크기가 바뀔 때처럼 이미 있는 차트의 영역을 그 자리에서
  옮긴다. `with_overlay(y_axis, &series_ids)`는 오버레이 레이어를 더하고,
  `with_legend_swatch(label, color)`는 시리즈가 아닌 의미를 나타내는 색 사각형 범례 항목을 더한다.

```rust
use renderer::{Annotation, AnnotationAxis, Chart, color::Color, config::AxisOptions};

let red = Color::from_rgb8(200, 60, 50);
let chart = Chart::sized(960, 600)
    .with_overlay(AxisOptions::overlay_y("Pressure (kPa)", red), &["pressure"])
    .with_annotation(Annotation::reference_line(AnnotationAxis::Y, 0.0, Color::BLACK, 1.0))
    .with_annotation(Annotation::span(AnnotationAxis::X, 2.0, 3.5, red.with_alpha(0.15)))
    .without_grid();
```

### 축 단위

데이터는 호스트가 등록한 그대로 두고 축이 값을 다른 단위로 찍게 할 수 있다
([#273](https://github.com/shim9610/figgy/issues/273)). `AxisOptions::unit: Option<AxisUnit>`를
정하면 눈금 라벨이 `scale * value + offset`을 찍는다. 16비트 ADC 원시 카운트를 볼트로, 켈빈을 섭씨로,
초를 밀리초로 읽게 하는 데 쓴다.

- 눈금은 표시 단위로 고르므로 변환한 뒤에도 라벨이 딱 떨어지는 수(`0, 1, 2 V`)이고, 각 눈금은 데이터
  위치 `(shown - offset) / scale`에 놓인다. `major_spacing`은 표시 단위로 읽는다.
- `min`, `max`, 데이터, `breaks`, `cross_at`, 주석, 피킹 결과는 데이터 단위 그대로다.
- 라벨 형식은 표시 값에 적용한다. 로그축은 `offset = 0`, `scale > 0`만 받고 표시 값의 10배 간격에 눈금을
  놓는다. 확률 축, `Timestamp` 라벨 축, 범주형 축은 단위를 거부한다.
- `AxisOptions::unit(scale, offset)`(`helpers`)은 단위를 정하고 표시 범위에 맞는 간격을 고른다. 주 축은
  필드를 정한 뒤 범위를 정한다. `set_y_range`와 `auto_fit_*` 계열이 표시 범위의 간격을 고른다.
- `Chart::with_x_unit(scale, offset)`과 `Chart::with_y_unit(scale, offset)`(`helpers`)은 `bottom_x`와
  `left_y`에 같은 일을 한다. 단위를 정하고 표시 범위에 맞는 간격을 고르므로 범위를 먼저 정해도 되고
  나중에 정해도 된다. `with_label_format`처럼 이 두 축만 바꾸므로 `top_x`, `right_y`, 오버레이 축에는
  다른 단위를 줄 수 있다. 축이 거부하는 단위는 그대로 저장되고 `Config::validate_chart`가 그 이유를
  알려 준다. 패닉은 일어나지 않는다.

```rust
let mut chart = Chart::sized(960, 600);
chart.set_x_range(250.0, 350.0);
chart.set_y_range(0.0, 32768.0);
let chart = chart
    .with_x_unit(1.0, -273.15)                         // 켈빈을 섭씨로
    .with_y_unit(5.0 / 32768.0, 0.0);                  // 카운트를 볼트로
```

```rust
use renderer::{Chart, color::Color, config::{AxisOptions, AxisUnit}};

let mut chart = Chart::sized(960, 600)
    .with_y_title("Voltage (V)")
    .with_overlay(
        AxisOptions::overlay_y("Temperature (°C)", Color::from_rgb8(176, 64, 32))
            .unit(1.0, -273.15),                       // 켈빈을 섭씨로
        &["temperature"],
    );
chart.config_mut().left_y.unit = Some(AxisUnit::new(5.0 / 32768.0, 0.0)); // 카운트를 볼트로
chart.set_y_range(0.0, 32768.0);                       // 0, 1, 2, 3, 4, 5 V에 눈금
```

### 라벨 속 수식

텍스트를 `&str`로 받는 편의 함수는 matplotlib이 라벨을 읽듯 그 텍스트를 읽는다. `Chart::with_title`,
`with_x_title`, `with_y_title`, `with_legend_entry`, `with_colorbar_title`, `with_legend_swatch`,
`SeriesConfig::label`, `Annotation::text`, `AxisOptions::overlay_y`, `RadialChart::with_title`,
그리고 브라우저의 텍스트 설정 함수([WASM.md](WASM.md#라벨-속-수식))가 여기에 해당한다.

- `$…$`와 `\(…\)` 구간은 본문 스타일의 TeX, `$$…$$` 구간은 디스플레이 스타일의 TeX다. 구간 밖은
  일반 텍스트다.
- `\$`는 달러 기호다. 예: `r"Price (\$)"`.
- TeX를 해석할 수 없는 텍스트는 쓴 그대로 보이고, `\$`만 `$`로 바뀐다. 편의 함수는 실패하지 않는다.

같은 읽기 방식을 `text::rich_segments_from_label`로 직접 쓸 수 있다(`RichText` 전체가 필요하면
`RichText::from_label`). 문서 타입은 명시적으로 남는다. `RichText::plain`은 일반 텍스트이고,
`RichText::from_tex`는 해석하지 못한 바이트 위치를 담은 `TexError`를 돌려준다.

```rust
use renderer::{Chart, SeriesConfig, color::Color, text::RichText};

let fit = SeriesConfig::line("fit", "t", "fit").label(r"$\hat y \pm 1.96\sigma$");
let chart = Chart::sized(960, 600)
    .with_x_title("Time (s)")
    .with_y_title(r"$\sin(2\pi f t)$")
    .with_legend_from(&[fit]);

// 저수준: 문서의 텍스트를 명시적으로, 검사하며 만든다.
let title = RichText::from_tex(r"$E = mc^2$", Color::BLACK, 18.0, "")?;
```

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
- **삼각 메시**: `DataRenderType::TriMesh`는 호스트가 준 삼각형 위에 색 채움과 등고선을
  그리고, 3D 차트에서는 상자 안의 표면으로 그린다. 정점 좌표와 값은 컬럼으로 올리고 삼각형은
  `Renderer::upsert_triangles(id, &TriangleSource { indices })`로 올린다(삼각형마다 정점
  번호 세 개, `Column<u32>`). 렌더러는 삼각분할을 하지 않는다. [가이드](TRIMESH.md)와
  플레이그라운드의 `TriMesh`, `TriSurface` 장면.
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

### 단조 x 선

간격이 일정하고(`x = start + i × interval`) 빠르게 쌓이는 대용량 데이터(데이터 수집 장비의
흐름 등)는 `SeriesConfig::monotonic_line(id, group, channel)`로 묶음의 채널 하나를 M4
엔벌로프로 그린다. 픽셀 열마다 첫 샘플, 가장 낮은 샘플, 가장 높은 샘플, 마지막 샘플만
남기므로, 프레임 비용은 샘플 수가 아니라 플롯 너비와 채널 수로 정해진다.
샘플은 호스트가 가진다. 호출마다 `MonotonicSource`로 빌려주며(채널별 슬라이스는
`MonotonicSlices`, 간격을 둔 섞인 버퍼 하나는 `MonotonicInterleaved`, 원소 형식은 `f64`,
`f32`, `i32`, `i16`, `u16`, `u8`), 호출이 끝난 뒤에는 소스를 붙잡아 두지 않는다.

```rust,ignore
use renderer::renderer::{MonotonicGroupRef, MonotonicRange};
use renderer::layout::Side;
use renderer::{MonotonicAppend, MonotonicSlices, SeriesConfig};

let series = vec![
    SeriesConfig::monotonic_line("ch0", "daq", 0).label("channel 0"),
    SeriesConfig::monotonic_line("ch1", "daq", 1).label("channel 1"),
];
// 단조 시리즈가 있는 차트는 단조 차트이며 그 핸들로만 닿는다. 일반 차트 호출은 받지 않는다.
let id = renderer.register_monotonic_chart(chart.config().clone(), series)?;
// 소스가 샘플과 start, interval을 담는다. 전체 호출은 전체를 축약한다. `Autoscale`은 x 범위를
// 샘플에, y 범위를 이번 축약이 남긴 값에 맞추고, `X { min, max }`는 x 범위를 정한다.
let channels = [ch0.as_slice(), ch1.as_slice()];
let source = MonotonicSlices::new(0.0, 1e-3, &channels);
let mut daq = renderer.monotonic_chart(id)?;
daq.set_monotonic("daq", &source, MonotonicRange::Autoscale)?;
// 그다음에는 새 샘플만 넘긴다(추가는 전체 호출의 격자를 이어 간다). `sliding`은 창 길이를
// 유지하고 x 범위를 새 샘플만큼 옮긴다.
let fresh = [new0.as_slice(), new1.as_slice()];
daq.append_monotonic("daq", &MonotonicSlices::new(0.0, 1e-3, &fresh), MonotonicAppend { sliding: true })?;
// 확대는 새 범위로 하는 전체 호출이다.
daq.set_monotonic("daq", &source, MonotonicRange::X { min: 0.0, max: 2.0 })?;
// 크기 조정(차트 영역, 또는 데이터 영역 크기 조정이 제안한 플롯)과 배치 맞춤도 소스를 받는다.
let groups = [MonotonicGroupRef::new("daq", &source)];
daq.resize(new_chart_area, None, Some((width, height)), &groups)?;
daq.fit_layout(&groups)?;
// 나머지는 소스 없이 편집기로 문서를 고친다.
daq.edit_config(|config| {
    config.chart_title().visible = true;
    config.axis(Side::Left).title().visible = true;
})?;
```

x에서 픽셀로의 매핑(x 범위, 장치 픽셀 단위 플롯 너비, x 방향)을 옮기는 호출은 이 넷뿐이고
모두 소스를 받는다. 일반 차트 호출은 `ChartId`를 받는데 단조 차트는 이를 내주지 않으므로 단조
차트에 닿지 못한다. `register_chart`와 시리즈 호출은 단조 선 시리즈를 거부한다
(`RuleKind::MonotonicGeneralChart`). 문서 편집기(`MonotonicConfig`)는 범례, 제목, 축 제목,
눈금, 눈금 라벨과 형식, 색, 주석, 선택, 꾸밈 요소 끌기를 고치며 x 범위, x 스케일, 플롯 영역을
쓰는 메서드는 없다. 이를 옮길 수정은 거부한다(`RuleKind::MonotonicMappingEdit`). 선 굵기는
일반 스타일 수정이다(`set_series`). 굵기가 장치 픽셀 약 120까지인 선은 가장자리를 정확히 그리고,
더 굵은 선은 플롯 가장자리에서 일부가 빠질 수 있다. 프레임과 피킹에는 소스가 필요 없다. `renderer.monotonic_needs_source(id)`는 묶음
데이터를 한 번도 받지 않은 시리즈를 알려 준다. `prepare_monotonic`은
`MonotonicChartDrawItem`을 그리고, 내보내기는
`ExportSource::Monotonic { chart: id, monotonic: &groups }`를 받는다(화면 배율이면 `&[]`).
피킹 결과는 남긴 점의 논리 샘플 번호다. 로그·확률·끊긴 x 축은 거부한다.
[MONOTONIC.md](https://github.com/shim9610/figgy/blob/master/crates/model/MONOTONIC.md)를 참고한다.

### 박스플롯

`DataRenderType::BoxPlot` 시리즈는 범주형 축 위에 미리 집계한 사분위수·중앙값·수염·평균·이상치를
상자 통계 레코드와 이상치 가변 길이 컬럼에서 읽어 그린다.
무광·새틴 금속 질감, 개별 스타일, 그룹 배치, 가로·세로 방향을 지원한다.
노치는 입력받은 중앙값 신뢰구간으로 그린다. 예전 `BoxPlotChart` 문서는 `Config::from_boxplot`으로 연다.
[사용법](BOXPLOT.md)을 참고하거나 `cargo run -p figgy-renderer --example boxplot_gallery`로 예제를 실행한다.
선택과 제목·축·라벨·범례 편집은 직교 좌표 차트와 같다([공통 선택·이동 API](INTERACTION.md)).

### 레이더 차트

레이더 차트는 각도 축이 범주형인 극좌표 차트다. 범주마다 스포크 하나, 숫자 반지름 축 하나를
공유하며, `DataRenderType::Radar` 시리즈는 채움과 꼭짓점 마커를 갖춘 닫힌 외곽선으로 그린다. 고리는
다각형(거미줄, `PolarOptions.grid_shape`) 또는 원으로 그린다. 범주·시리즈마다 값 하나를 호스트가
컬럼으로 넣으며 스포크별 정규화는 계산하지 않는다. 예전 `RadarChart` 문서는 `Config::from_radar`로 연다.
[사용법](RADAR.md)을 참고하거나
`cargo run -p figgy-renderer --example radar_gallery -- target/radar-gallery`로 예제를 실행한다.
선택과 제목·축·라벨·범례 편집은 극좌표 차트와 같다([공통 선택·이동 API](INTERACTION.md)).

### 극좌표 플롯

극좌표 플롯은 `config.set_polar(Some(PolarOptions { .. }))`를 켠 직교 차트다. 직교 차트와 같은
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
컬럼에서 읽으며, 스트리밍 소스는 거부한다. 막대별 색·재질·둥근 모서리·외곽선은 시리즈에 막대별로
덮어쓰는 값이고, 막대 하나는 히스토그램 구간처럼 선택한다. 예전 `CategoricalChart` 문서는
`Config::from_categorical`로 연다.
[API 사용법](CATEGORICAL.md)을 참고하거나
`cargo run -p figgy-renderer --example categorical_gallery -- target/categorical-gallery`로 예제 PNG를 만든다.

### 원형 차트

`RadialRenderer`는 파이·도넛 차트와, 한 조각을 옆의 작은 원으로 펼치는 분할 원(pie-of-pie)을
그린다. 무광·세라믹·새틴 금속·툰 등의 재질, 둥근 모서리, 외곽선, 호버·선택을 지원한다.
조각별 색·스타일·표기 내용은 `RadialChart` 모델에서 직접 편집한다. 호스트의 wgpu 장치를 공유하며
컬럼 풀이나 컬럼 스트림은 쓰지 않는다.
[네이티브 편집기](examples/radial_editor.rs)는
`cargo run -p figgy-renderer --example radial_editor --features egui_demo`로 실행한다.
[사용법과 예제](RADIAL.md)에서 [옵션 기본값과 범위](RADIAL.md#스타일-옵션-기본값과-범위),
[조각별 편집과 상속](RADIAL.md#ssot-편집과-선택), 프레임 수명과 내보내기를 포함한
[호스트 갱신 순서](RADIAL.md#호스트의-갱신-순서)를 확인할 수 있다.

<a id="png-export-메모리-only--저장은-caller"></a>

### 창 없는 렌더러와 형식별 저장

`helpers` 기능을 쓰면 창 없는 렌더러를 한 번의 호출로 만들고, 파일 이름이 가리키는 형식으로
내보내 저장하는 것도 한 번의 호출로 끝난다. 단계는 그대로 드러난다. 만들기, 올리기, 문서 정하기,
저장하기 순서다.

```rust
use renderer::{Chart, ExportOptions, ExportSource, Renderer, SeriesConfig};

let mut renderer = Renderer::headless_blocking()?;        // 인스턴스, 어댑터, 장치, 파이프라인
renderer.add_column("x", &xs)?;                          // Vec<f64>나 슬라이스를 감싸지 않고 넘긴다
renderer.add_column("y", &ys)?;
let series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
let mut chart = Chart::sized(960, 600).with_title("A sine wave").with_legend_from(&series);
chart.fit_layout();                                       // 여백을 라벨에 맞춘다
let source = ExportSource::Chart { chart: &chart, series: &series };
renderer.save_blocking(source, ExportOptions::document(2.0), "sine.png")?;  // .svg, .pdf도 된다
```

- `Renderer::headless_blocking()`(네이티브)과 `Renderer::headless(options).await`(브라우저 포함)은
  `WGPU_BACKEND`를 따른다. `HeadlessOptions`로 처음 컬럼 풀 크기(기본 16 MiB)와 전력 선호를
  정한다(`headless_blocking_with`).
- `ExportFormat`은 `Png`, `Svg`, `Pdf`다. `ExportFormat::from_path`는 확장자를 대소문자 구분 없이
  읽는다. `export_bytes`(비동기)와 `export_bytes_blocking`은 바이트를 돌려준다. `save_blocking`은
  확장자로 형식을 고르고 `save_as_blocking`은 형식을 직접 받는다. 모두 아래의 내보내기 함수를 그대로
  부르므로 파일 내용은 그 내보내기 결과와 같다.
- 저장이 실패하면 `SaveError`를 돌려준다. 내보내기 자체의 오류, 형식을 알 수 없는 확장자, 파일 쓰기
  오류 중 하나다.
- `RadialRenderer`에도 같은 `export_bytes`, `export_bytes_blocking`, `save_blocking`이 있고, 차트와
  크기를 받는다.
- `Chart::sized(width, height)`는 기본 모양에 그 크기의 차트 영역을 준 차트다.
  `with_legend_from(&series)`는 라벨이 있는 시리즈마다 그 시리즈의 기호로 범례 항목을 하나씩 더하고,
  `fit_layout()`은 축 띠와 여백을 라벨과 제목에 맞춘다(`layout::auto_layout`).
- `RadialRenderer::headless_blocking()`은 원형 차트 렌더러를 같은 방식으로 만든다.
  `RadialChart::with_title(text)`는 차트 자체의 글꼴과 라벨 색으로 제목을 주고,
  `RasterImage::save_png(path)`는 `export_rgba`로 받은 이미지를 PNG 파일로 쓴다. 저장하기 전에
  픽셀을 검사하려는 호스트가 쓴다.
- 차트 영역의 위치, 오버레이, 주석, 서브플롯은 [차트 부품](#차트-부품)에 정리했다.

### 테마

테마(`renderer::theme::Theme`, `helpers` 기능)는 차트 전체의 모양을 문서에 한 번 써 넣는다.
문서에 이름으로 저장되는 것이 아니라 값이다. 적용하고 나면 문서에는 평범한 값만 남고, 문서가 계속
하나뿐인 원본(SSOT)으로 남는다.

```rust
use renderer::theme::{SubplotScope, Theme};

let mut series = [SeriesConfig::line("sine", "x", "y").label("sin x")];
let mut chart = Chart::sized(960, 600).with_title("A sine wave").with_legend_from(&series);
chart.apply_theme(&Theme::dark(), &mut series, SubplotScope::All);  // 레이아웃도 다시 맞춘다
```

- 내장 테마는 `Theme::light()`(문서에 적힌 기본 모양), `dark()`, `paper()`(열린 축 틀, 격자 없음,
  흑백 인쇄에 안전), `dashboard()`(축선·눈금 없음, 가로 격자, 큰 글자), `sketch()`(손으로 그린 선)다.
  `Theme::named("dark")`로 이름으로 찾을 수 있다. 모든 필드가 공개되어 있어서 복사본을 고친 뒤
  적용해도 된다.
- 테마가 쓰는 것은 크롬(`ChromeTheme`)이다. 차트와 플롯 배경, 그리기 스타일, 축 틀(`AxisPreset`),
  축·눈금 라벨·제목·범례의 색과 글꼴과 크기, 격자, 범례 상자, 색상 막대 테두리, 선택 강조가
  여기에 든다. 시리즈마다 계열별 부분(`SeriesTheme`)도 쓴다. `ColorCycle`에 따른 색, 선 굵기,
  마커 크기, 계열별 재질이다. 에너지 준위 차트와 파인만 도형은 자기 축 틀(`Grotrian`, `Hidden`)을
  유지한다.
- 축 범위, 스케일, 눈금 간격, 라벨 형식, 범주, 축 끊김, 차트 영역, 모든 오프셋, 모든 텍스트
  내용, 주석 레이어, 오버레이와 서브플롯 구조, 시리즈 id·컬럼·라벨, 선택 자체는 그대로 둔다.
- 오버레이는 차트와 함께 테마를 받는다. 서브플롯은 `SubplotScope`로 고른다. `None`, `All`,
  `Only(&[인덱스])` 중 하나이고, 범위 밖 서브플롯에 그려지는 시리즈는 모양을 유지한다.
- `Chart::apply_theme`은 바뀐 글자 크기에 맞춰 레이아웃을 다시 맞춘다(`fit_layout`).
  `Chart::with_theme(&theme)`는 크롬만 적용하고 맞추며, 시리즈는
  `theme.style_series(&mut series, index)`로 꾸민다. `theme.apply(&mut config, &mut series, scope)`는
  `Config`에 바로 적용하고 레이아웃은 호스트에게 맡긴다. `theme.apply_radial(&mut radial_chart)`은
  원형 차트에 적용한다.
- 같은 테마를 두 번 적용해도 더 바뀌는 것은 없다.

### PNG 내보내기

```rust
let bytes = renderer
    .export_png(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::panel(scale))
    .await?;
std::fs::write("/tmp/out.png", &bytes)?;          // 클립보드나 네트워크로 보내도 된다.

// RGBA만 필요하면:
let img = renderer
    .export_rgba(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::panel(scale))
    .await?;
// img.width, img.height, img.rgba (straight alpha, 길이 = w * h * 4)
```

네이티브 호스트는 같은 인자로 `export_png_blocking`과 `export_rgba_blocking`도 호출할 수 있다.
파일 저장은 호출자가 담당한다.
예제의 `series_configs`는 그릴 `SeriesConfig` 슬라이스이고(앞의 초기화 예제라면
`std::slice::from_ref(&cfg)`), `scale`은 `2.0` 같은 출력 배율이다. 결과 이미지는 차트 영역에
`scale`을 곱한 크기이며 배경은 완전히 투명하다. 배경색을 넣으려면
`ExportOptions::panel(scale).with_clear(Color::WHITE)`를 넘긴다.
패널 내보내기(`ExportOptions::panel`, `ExportScope::Panel`)는 기본 차트 영역만 그린다. `overlays`나
`subplots`가 있는 차트에 `ExportOptions::document(scale)`를 주면 레이어와 서브플롯을 이미지 한 장에
모두 그린다. 이 이미지는 모든 차트 영역을 합친 크기다.
`scale`의 최솟값은 `renderer::MIN_EXPORT_SCALE`(0.25)이고 NaN이나 0 이하의 값은 0.25로 바꾼다. 상한은 없다. 배율을 적용한 이미지가 장치의 텍스처 크기 한도(`max_texture_dimension_2d`, 보통 한 변 8192 또는 16384픽셀) 안에 들어와야 하며, 넘으면 메모리를 잡기 전에 `FiggyError::GpuResourceLimit` 오류를 돌려준다. DPI에서 배율을 구하려면 표준값 96 DPI를 기준으로 계산하는 `renderer::dpi_to_scale(dpi)`를 사용한다.

폰트, 선, 여백, 격자, 범례의 픽셀 크기가 모두 같은 비율로 바뀌므로 화면과 같은 배치를 더 높은 해상도로 얻는다. 1.0보다 작은 배율은 해상도를 낮춘다.

### SVG와 PDF 내보내기

```rust
let svg: String = renderer
    .export_svg(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::document(scale))
    .await?;
let pdf: Vec<u8> = renderer
    .export_pdf(renderer::ExportSource::chart(&chart, &series_configs), renderer::ExportOptions::document(scale))
    .await?;
std::fs::write("/tmp/out.svg", svg)?;
std::fs::write("/tmp/out.pdf", pdf)?;
```

두 문서에는 같은 인자로 `export_png`를 호출했을 때와 같은 그림이 담긴다([#48](https://github.com/shim9610/figgy/issues/48)).
장식 요소는 PNG를 그릴 때와 같은 배치에서 벡터로 기록한다. 축, 눈금, 눈금 라벨, 격자, 극좌표의 원과 방사선, 축 제목과 차트 제목, 범례, 색상 막대의 테두리·축·라벨, 주석 층이 여기에 해당한다. 글자는 글리프 윤곽선으로 기록하므로 글꼴을 포함하지 않으며, 글자를 선택하거나 검색할 수 없다.
데이터 층(채움, 선, 마커, 막대, 필드, 등고선, 벡터, 선택 강조, 값 라벨)은 `scale`배 해상도의 이미지로 넣으며, 이미지의 픽셀은 PNG 출력의 픽셀과 같다. 차트와 그 오버레이가 이미지 한 장이고, 서브플롯마다 한 장씩 더해진다. 에너지 준위 도표처럼 설정만으로 정의되는 시리즈도 PNG와 같은 배치로 벡터로 기록한다([#234](https://github.com/shim9610/figgy/issues/234)). 이 시리즈는 아래쪽의 다른 데이터와 위쪽 값 라벨 사이에 놓이므로, 이런 시리즈가 있는 차트나 서브플롯은 아래 데이터 이미지, 시리즈, 라벨 이미지 순으로 기록한다. 다른 시리즈가 하나도 없는 이미지는 넣지 않는다. 색상 막대의 색 띠와 Milkyway 배경도 이미지로 넣으며, Milkyway의 발광 효과는 그리지 않는다. PNG 출력과 마찬가지로 선택 상자는 그리지 않는다.

SVG의 `viewBox`는 실제 픽셀 단위이고 `width`·`height`는 PDF 페이지처럼 논리 크기를 포인트로 나타낸다(1 px = 0.75 pt). 따라서 2배 출력은 페이지 크기가 같고 이미지 해상도만 두 배가 된다. `scale`은 PNG 출력과 같은 범위로 제한한다. `ExportOptions::with_clear`로 정한 배경색은 맨 아래에 사각형으로 그린다. 두 출력은 문서 전체를 받으며 `ExportOptions::panel`은 `FiggyError::InvalidConfig`로 거부한다. 출력 함수는 모두 비동기이고, 네이티브 호스트는 `_blocking` 함수도 쓸 수 있다([GPU 결과](#gpu-결과-비동기-동기-확인)). `WindowedRenderer`에도 같은 메서드가 있다. 스트리밍 원본을 쓰는 시리즈는 PNG 출력처럼 `FiggyError::ColumnNotResident`로 거부한다.

원형 차트는 `RadialRenderer::export_svg(&chart, size, options)`와 `export_pdf`(동기 버전 `export_svg_blocking` / `export_pdf_blocking`)로 내보낸다. 조각과 배경은 이미지 한 장이고, 제목, 라벨과 그 지시선, 분할 원을 잇는 선, 주석은 벡터로 기록한다. 내부 라벨은 모두 PNG에서와 같은 위치에 모든 조각 위로 그리며, 선택 상자는 그리지 않는다.

---
