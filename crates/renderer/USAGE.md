# Native Rust usage / Rust 사용법

[Start here](https://github.com/shim9610/figgy/blob/master/README.md#quick-start) · [한국어 시작하기](https://github.com/shim9610/figgy/blob/master/README.ko.md#빠른-시작)

## 1. Usage

### Toolchain and builds

This release pins **Rust 1.99.0** in
[`rust-toolchain.toml`](https://github.com/shim9610/figgy/blob/master/rust-toolchain.toml), including the wasm32 target,
rustfmt and clippy. With rustup installed, commands in this checkout select
that toolchain automatically. The package manifests declare Rust 1.99 as the
support floor; earlier toolchains have not been validated. Use the committed
`Cargo.lock` with `--locked` when building this checkout; downstream Git
consumers resolve dependencies with their own lockfile.

```bash
cargo check --locked --workspace --all-targets --all-features
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
```

### Package names and compatibility

Development packages are now named `figgy-renderer` and `figgy-model`; Rust imports
remain `renderer::...` and `model::...`. When moving a Git dependency to a revision
with this rename, add the corresponding `package` key while retaining its existing
dependency alias. Older public revisions still use their original package names.
This checkout targets `figgy-renderer` 0.17.0 and `figgy-model` 0.11.0. The
dependency examples below use these versions and apply once they are published;
use a local path to try a checkout before publication.

Use the renderer's optional `serde` feature for serialization of its re-exported
model types; no direct model dependency is needed for that. Rust 1.99 is the
explicit support floor and direct host GPU sharing requires wgpu 30. The WASM
wrapper remains a separate consumer and is excluded from Rust registry publishing.
See [Rust API contracts](API.md) for ownership, frame lifetimes,
render targets and compatibility policy.

For selective edits, change only the desired fields in the current `Config`
(the SSOT). For example, write axis `min` / `max` through
`Chart::with_axis_range_change` to preserve tick settings. Convenience helpers
such as `set_x_range`, `auto_fit_*` and `with_title` also adjust related ticks,
labels or title layout. See the [operation table and range-only example](API.md#direct-ssot-edits-and-convenience-helpers)
before choosing an edit path.

### Adding the dependency

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.17.0" }
wgpu     = "30"
```

For a local checkout, replace `version` with a `path` relative to the consuming
application's `Cargo.toml`. A standalone program that calls `pollster::block_on`
(as the `line` example does) must also declare `pollster = "1.0"`. Crate examples
already have access to this dependency through the renderer manifest.

For the full model-edit → prepare → pick → redraw → export sequence, including
the distinction between logical and physical pointer coordinates, see
[the native API workflow](API.md#native-edit-pick-and-export-workflow).

Cartesian hosts own the derived `ChartView` and, with `prepare`, the series'
`ChartStyle` values. After changing axes, grid, labels, legend or panel size, call
`refresh_axis` (or `refresh_axis_with_selection`) before preparing. Recreate
affected styles after series appearance/mapping/identity changes, using the same
display scale. `prepare` updates transforms and GPU data but does not perform
those two steps. The registered-chart path maintains its own style cache after
`set_chart_config` / `set_chart_series`; its caller-owned view still needs refresh.
See the [edit table and compiled example](API.md#cartesian-edits-update-the-derived-inputs-before-preparing).

The refresh also reports tick settings the axis raster could not draw as written.
Read `view.render_warnings()` in the same branch that calls `refresh_axis`: each
`RenderWarning` names the axis, the value as written and the value drawn, such
as a `major_spacing` that would need more than 1000 intervals and was drawn at
automatic spacing. The list is re-derived only when the view is rasterized, so a
host that refreshes on its dirty flag never pays for it per frame. Compare it
with the previous list to show only changes. The same list ends with the
annotation layer's `AnnotationSkipped` warnings: an edit of
`config.annotations.items` is a third dirty source,
`ChartRenderStamp::needs_annotation_raster_since`, answered by
`refresh_annotations`, which rewrites only the two annotation textures (see
[the annotation layer](INTERACTION.md#annotation-layer)).

Constructors initialize base resources; they do not eagerly populate every
pipeline cache. Use explicit `prewarm_all` or detached preparation when needed,
following the [constructor/prewarm table](API.md#construction-and-prewarming).
`set_memory_budget` is a policy for checked allocations, not a hard VRAM ceiling;
the [budget contract](API.md#gpu-memory-accounting-and-budget-scope) lists its
initial-pool behavior, tracked lifetime and exclusions.

The default library features require neither winit, egui nor iced. Pull in only
the host you actually use; `egui_demo` enables the optional egui example dependencies:

```toml
# winit standalone
winit = "0.30"

# egui embedded
eframe    = { version = "0.36", default-features = false, features = ["wgpu", "x11", "wayland"] }
egui      = "0.36"
egui-wgpu = "0.36"
```

iced 0.14 still uses wgpu 27, so direct device/queue/render-pass sharing is
disabled until iced publishes a wgpu 30-compatible release.
The X11/Wayland features above enable Linux window creation; select the Linux
window backends appropriate for your host.

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
// polars Series, mmap, anything — native upload writes directly into mapped staging
// without a conversion Vec. Built-in `Column<f64>` works too.
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

`Renderer::add_column` takes `&dyn ColumnSource` — implement the trait on any container of yours and native upload writes into mapped staging without an intermediate conversion `Vec`. The GPU then copies staging into the column pool; “zero-copy” here means no extra CPU conversion buffer or retained CPU point mirror. The source writes GPU pairs and returns smallest-positive statistics in the same pass; the renderer never reads wgpu 30's write-only mapped bytes. `min` / `max` retain their source-level meaning. Scalar smallest-positive uses the actual uploaded `(value as f32, 0)` value, while hi-lo uses the recorded `hi as f64 + lo as f64`; both include only finite positive values. Use `Renderer::add_hilo_column` with `&dyn HiLoColumnSource` for large absolute timestamps or coordinates that must preserve sub-f32 deltas.

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
cargo run -p figgy-renderer --example winit_simple
cargo run -p figgy-renderer --example egui_embed --features egui_demo
```

Each example shows:
- A 3-panel grid with different grid options (off / major / major + dotted minor)
- The RC panel renders 2 series (charging + discharging)
- Line widths of 1 / 2 / 3.5 px across panels
- Legends
- DPI input + Save PNG button (egui) or `S` key (winit) → per-panel PNG bytes in memory → written by the example to `/tmp/figgy_*_panel_{i}.png`

### Browser integration example — timestamp axes

`crates/web/timestamp-demo.html` exercises the browser timestamp path with
absolute Unix time values uploaded through `register_column_f64(Float64Array)`
and explicitly replaced through `update_register_column_f64`.
It lets you change the visible time window, data unit, timezone, fractional
second policy, label pattern, chart width, and export scale while the x axis
uses `LabelFormat::Timestamp` + `AutoCalendar` to choose non-overlapping labels.

```bash
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
cd crates/web
python -m http.server 8142 --bind 127.0.0.1
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

Full version: [examples/egui_embed.rs](examples/egui_embed.rs).

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
  domain (a bubble chart). Colour and radius are derived in the shader on
  every draw; nothing per point is stored.
  [Reference](CONFIG.md#point-colour-and-size-maps) and `value_maps`.
- **Vector fields**: `DataRenderType::Vector` draws one arrow per sample from
  (u, v) columns at scattered anchors or from two matrices on a grid, in
  pixel or data scale, optionally coloured through the colourbar.
  [Reference](CONFIG.md#vector-fields) and `vector_field`.
- **Overlay axes, subplots and annotations**: an overlay layer draws series
  against a second Y axis; subplots arrange whole charts in a grid with
  linked or twin axes; the annotation layer adds reference lines, spans,
  text and arrows around any family's frame. [Overlays](INTERACTION.md#overlay-layers),
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

`BoxPlotRenderer` draws precomputed quartiles, medians, whiskers, means and outliers
with matte/satin surfaces, individual styles, grouped layouts and either direction.
Notches use explicit median confidence intervals. See the [box plot guide](BOXPLOT.md)
or run `cargo run -p figgy-renderer --example boxplot_editor --features egui_demo`.
Titles, axes, labels and legends use the [shared interaction API](INTERACTION.md).
Studio integration and deployment are separate.

### Radar charts

`RadarRenderer` compares a few series over a fixed set of categories: one spoke
per category, one shared value axis, closed polygons with fills, outlines and
vertex markers on a polygon or circular grid. The host supplies one value per
category and series; per-spoke normalization is not computed. See the
[radar guide](RADAR.md) or run
`cargo run -p figgy-renderer --example radar_editor --features egui_demo`.
Titles, labels and legends use the [shared interaction API](INTERACTION.md).

### Polar plots

A polar plot is a Cartesian chart with `config.polar = Some(PolarOptions { .. })`:
the same line, scatter and area series, column pool, picking and export, drawn
in (angle, radius). `bottom_x` is the angle axis and `left_y` the radius axis;
the grid becomes rings and spokes, the angle axis the outer ring and the radius
axis a labelled ray that drags around the centre. Angles are degrees or radians,
the circle can be a sector with a hole, and segments are chords or arcs. Series
without a polar form (histograms, heatmaps, contours, error bars, vectors) and
streamed sources are rejected with an explicit error. See the
[polar guide](POLAR.md) or run
`cargo run -p figgy-renderer --example polar_plot -- target/polar-plot`.

### Categorical bars

`CategoricalRenderer` draws grouped, stacked and 100% stacked bars in either direction.
The bounded model owns category IDs, series and optional values separately from `ColumnSource`.
Per-bar colors, materials, rounding, outlines, label formats and selection are editable in the SSOT.
See the [categorical API guide](CATEGORICAL.md) or run
`cargo run -p figgy-renderer --example categorical_editor --features egui_demo`.
Studio integration and deployment are separate.

### Radial charts

`RadialRenderer` draws resident pie, donut, and pie-of-pie charts, with procedural
matte, ceramic, satin/brushed-metal, toon, and other surfaces. Rounded sector corners,
outlines, hover/selection, per-slice colors and styles, and label content live in the
SSOT model. It shares the host wgpu device and does not use column streams.
Try the [native editor](examples/radial_editor.rs) with
`cargo run -p figgy-renderer --example radial_editor --features egui_demo`. See the [radial chart API and example](RADIAL.md).
Studio integration and deployment are separate.
The guide covers [option defaults and ranges](RADIAL.md#스타일-옵션-기본값과-범위),
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
`scale` bounds: `renderer::MIN_EXPORT_SCALE` (0.25) ~ `renderer::MAX_EXPORT_SCALE` (8.0), automatically clamped.
Convert from standard 96 DPI via `renderer::dpi_to_scale(dpi)`.

When scaling, every pixel-based dimension (font / line / margin / grid / legend) scales proportionally → the visual is identical, just denser pixels.

---

---

## 1. 사용법

### 툴체인과 빌드

[`rust-toolchain.toml`](https://github.com/shim9610/figgy/blob/master/rust-toolchain.toml)에 개발 환경을 **Rust 1.99.0**으로 고정하고 wasm32 빌드 대상, rustfmt, clippy를 지정했다. rustup이 설치돼 있으면 저장소 안에서 명령을 실행할 때 자동으로 적용된다. 패키지 manifest에도 지원 하한을 Rust 1.99로 명시했다. 그보다 낮은 버전은 검증하지 않았다.
이 저장소를 빌드할 때는 `--locked`를 사용해 커밋된 `Cargo.lock`을 따른다. figgy를 Git 의존성으로 사용하는 프로젝트는 해당 프로젝트의 잠금 파일로 의존성을 결정한다.

```bash
cargo check --locked --workspace --all-targets --all-features
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
```

### 의존성 추가

Cargo 패키지 이름은 `figgy-renderer`·`figgy-model`로 정리했다. Rust의
`renderer::…`·`model::…` 경로는 유지한다. 이름을 바꾼 Git 리비전으로 갱신할 때는
Cargo 의존성에 `package`를 추가한다. 이전 공개 리비전은 기존 패키지명을 사용한다.
이 소스의 릴리스 대상은 `figgy-renderer` 0.17.0과 `figgy-model` 0.11.0이다.
아래 의존성 예제는 해당 버전이 게시되면 사용하며, 게시 전에는
`version` 대신 `path = "../figgy/crates/renderer"`처럼
애플리케이션 기준 경로를 지정해 현재 소스를 사용한다.

렌더러의 `serde` 기능을 켜면 재노출된 모델 타입을 직렬화할 수 있다. 이를 위해 모델을
직접 의존할 필요는 없다. 지원 Rust 하한은 1.99이며 GPU를 직접 공유하려면 호스트도
wgpu 30을 사용해야 한다. WASM 래퍼는 별도 소비 예제로 유지하고 Rust registry 게시에서는
제외한다. 소유권·프레임 수명·출력 타깃·호환성은 [API 계약](API.md)에 정리했다.

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.17.0" }
wgpu     = "30"
```

Default library features do not require winit, egui or iced; `egui_demo` enables
the optional egui example dependencies. Select only the host integration you use.

```toml
# winit standalone
winit = "0.30"

# egui 임베드
eframe    = { version = "0.36", default-features = false, features = ["wgpu", "x11", "wayland"] }
egui      = "0.36"
egui-wgpu = "0.36"
```

iced 0.14는 wgpu 27을 사용하므로, wgpu 30과 호환되는 iced 버전이 나올 때까지 장치·큐·렌더 패스를 직접 공유하는 통합은 사용할 수 없다.

<a id="standalone-초기화와-그리기-winit--figgy"></a>

### 독립 창에서 초기화하고 그리기 (winit + figgy)

다음 코드는 winit의 `ApplicationHandler::resumed` 메서드 안에서 실행하는 예다. `event_loop: &ActiveEventLoop`을 인자로 받는다. 전체 이벤트 루프와 창·렌더러 상태를 보관하는 방법은 [winit_simple.rs](examples/winit_simple.rs)를 참고한다.

```rust
use std::sync::Arc;
use renderer::{
    Chart, ChartDrawItem, DataLineStyleConfig, DataRenderType, Renderer, Series, SeriesConfig,
    color::Color, default, layout::{ChartArea, Rect}, line::LineStylePreset,
};

let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes()).unwrap());
let size = window.inner_size();

// 한 줄 셋업 — instance/adapter/device/queue/surface/swap chain 모두 figgy 가 소유.
let mut renderer = Renderer::for_window(
    Arc::clone(&window),
    (size.width, size.height),
    16 * 1024 * 1024,   // GPU column pool 16 MiB
).unwrap();

// renderer.add_column 은 `&dyn ColumnSource` 받음.
// 본인 데이터 타입에 trait 구현 (아래 `ColumnSource` 섹션 참조) — Vec, ndarray,
// polars Series, mmap 등 어떤 출처든 native에서는 변환 Vec 없이 mapped staging에 직접 기록.
// 빌트인 `Column<f64>` 도 사용 가능.
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

`Renderer::add_column`은 `&dyn ColumnSource`를 받는다. 사용 중인 데이터 타입에 이 트레이트를 구현하면 네이티브 환경에서 중간 변환용 `Vec` 없이 매핑된 스테이징 버퍼에 직접 기록할 수 있다. 스테이징 버퍼에서 컬럼 풀로 옮기는 작업은 GPU가 수행한다. 여기서 제로카피란 별도의 CPU 변환 버퍼나 원본 점 데이터의 CPU 복사본을 유지하지 않는다는 뜻이다.

데이터 소스는 값을 GPU용 쌍으로 기록하면서 최소 양수 값도 함께 계산한다. 렌더러는 wgpu 30의 쓰기 전용 매핑 영역을 다시 읽지 않는다. `min` / `max`는 원본 데이터 기준을 유지한다. 최소 양수 값은 일반 컬럼에서는 실제 업로드한 `(value as f32, 0)`, hi/lo 컬럼에서는 기록한 `hi as f64 + lo as f64`를 기준으로 계산하며, 유한한 양수만 포함한다.

`add_column` / `add_hilo_column`은 새 ID를 등록할 때 사용한다. 기존 ID의 데이터를 교체하려면 `upsert_column` / `upsert_hilo_column`을 사용한다. `Renderer`는 새 풀 상태, 영향을 받는 차트의 리비전, 피킹 캐시와 정리 작업을 모두 준비한 뒤 한꺼번에 반영한다. 준비 중 오류가 나면 기존 상태를 유지한다.

호스트가 별도의 파생 상태를 관리한다면 `begin_upsert_*`가 반환한 가드의 임시 풀을 참조해 그 상태도 미리 준비할 수 있다. 이후 `commit`을 호출하면 추가 할당이나 실패 가능 작업 없이 변경을 확정한다. 피킹 자원의 갱신은 렌더러가 담당한다.

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

**기본 제공 구현체**: `Column<f64>`, `Column<f32>`, `Column<Option<f64>>` (null → NaN).

사용자 정의 `ColumnSource`는 값 기록과 통계 계산을 함께 수행하는 메서드를 반드시 구현해야 한다. 빠뜨리면 컴파일 단계에서 오류가 나므로 업로드 도중에야 문제를 발견하는 일을 피할 수 있다. 기록한 바이트를 다시 읽거나 부정확한 대체 경로를 사용하지 않는다. `HiLoColumnSource`도 같은 이름의 메서드에서 `(hi, lo)`를 기록하고, 두 값을 합쳐 계산한 통계를 반환해야 한다.

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

renderer.add_column("temperature", &my_series)?;   // ↘ mapped staging memory 에 직접 write, Vec 0
```

`f32` 데이터도 같은 경로를 사용한다. 값을 순회하며 `writer.write_pair(index, value, 0.0)`를 호출하면 된다. `ColumnPairWriter`는 매핑된 바이트 배열을 직접 노출하지 않고 값 쌍을 쓰는 기능만 제공한다. wgpu를 직접 다루거나 `dst.copy_from_slice(...)`로 우회하지 않아도 중간 버퍼 없이 풀에 업로드할 수 있다.

<a id="네이티브-example--사인--rc--cross-section"></a>

### 네이티브 예제 — 사인 곡선, RC 회로, 단면 그래프

```bash
cargo run -p figgy-renderer --example winit_simple
cargo run -p figgy-renderer --example egui_embed --features egui_demo
```

각 예제에서 다음 기능을 확인할 수 있다.

- 패널 3개에 서로 다른 격자 설정 적용: 격자 없음, 주 격자선, 주 격자선과 점선 보조 격자선
- RC 회로 패널의 충전·방전 곡선 두 개
- 패널별 선 두께 1 / 2 / 3.5px와 범례
- egui의 DPI 입력·Save PNG 버튼 또는 winit의 `S` 키로 패널별 PNG 저장. 예제는 메모리로 받은 PNG를 `/tmp/figgy_*_panel_{i}.png`에 저장한다.

<a id="브라우저-timestamp-축-데모"></a>

### 브라우저 통합 예제 — 시간축

`crates/web/timestamp-demo.html`은 Unix 시간 값을 `register_column_f64(Float64Array)`로 등록하고 `update_register_column_f64`로 교체하는 예제다. 시간 범위, 데이터 단위, 시간대, 소수 초 표시, 라벨 형식, 차트 너비, 출력 배율을 바꿔 볼 수 있다. X축에는 `LabelFormat::Timestamp`와 `AutoCalendar`를 사용해 눈금 간격을 자동으로 조절한다.

```bash
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
cd crates/web
python -m http.server 8142 --bind 127.0.0.1
# http://127.0.0.1:8142/timestamp-demo.html 열기
```

<a id="라이브-ssot-lab--풀-스케일에서-본-분리-api"></a>

### 실시간 설정 편집 예제 — 대용량 데이터와 분리된 렌더링 API

```bash
cargo run --release -p figgy-renderer --example ssot_lab --features egui_demo
```

2×2 패널에 정밀 점선·스케치·은하수·별자리 스타일을 하나씩 표시한다. 네 시리즈는 GPU 풀의 `x` 컬럼 하나를 공유한다. 사이드바에서 X축이 연동된 패널 쌍의 이동 방향, 표시 범위의 너비, 시리즈당 데이터 수를 바꿀 수 있다. 최대 데이터 수는 시리즈당 300만 개, 총 1200만 개이며 컬럼은 5개다.

설정 변경은 `Renderer::prepare`와 `Renderer::paint_prepared`를 거쳐 화면에 반영된다. `Mutex`나 프레임마다 별도로 호출하는 `update_transform`은 필요하지 않다. 상태창의 `frames skipped`로 편집 중 무효화된 프레임 토큰 때문에 그리기를 건너뛰었는지 확인할 수 있다.

### egui 통합 패턴 (요약)

호스트의 콜백 구조에 맞춰 프레임 준비와 명령 기록을 나눈다. 상태 변경은 `Renderer::prepare`가 `&mut self`로 수행하고, `Renderer::paint_prepared`는 `&self`로 그리기 명령만 기록한다. 따라서 paint 콜백에서 렌더러를 `Mutex`로 감쌀 필요가 없다.

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

호스트는 한 프레임의 명령 버퍼를 모두 GPU 큐에 제출한 뒤 `renderer.end_gpu_frame()`을 정확히 한 번 호출해야 한다. egui처럼 콜백 실행을 관리하는 호스트에서는 이전 프레임의 제출이 끝났음을 확인한 뒤 다음 프레임의 첫 prepare 직전에 호출해도 된다. 패널마다 호출하면 아직 제출하지 않은 다른 패널의 자원을 너무 일찍 회수 대상으로 넘길 수 있다.

`WindowedRenderer::draw*`나 패널 출력처럼 렌더러가 직접 제출하는 경로에서는 이 처리를 내부에서 수행한다. 외부 렌더 패스 기록과 이 경로를 함께 쓴다면 미리 기록해 둔 명령 버퍼를 먼저 모두 제출해야 한다. 렌더러는 wgpu 명령 버퍼의 내부 참조를 볼 수 없으므로 호스트가 아직 제출하지 않은 자원만 따로 골라 보호할 수 없다.

`paint_prepared`는 같은 토큰으로 여러 패스에 호출할 수 있다. `PreparedFrame`이 그리기에 필요한 입력을 보관하므로 paint 단계에서 `items`를 다시 만들거나 전달하지 않는다. 준비 후 관련 렌더러 자원이 바뀌면 아무 명령도 기록하지 않고 `FiggyError::StalePreparedFrame`을 반환한다. 다음 프레임에서 다시 `prepare`하면 된다.

자동 등고선 라벨도 패널·항목과 시리즈의 각 등장 위치별로 이 소유권 규칙을 따른다. 아틀라스와 셀 표는 변경하지 않는 캐시 자원으로 공유한다. 서로 다른 컴퓨트 입력은 파라미터, 좌표 변환, 후보·앵커·간접 실행 인자 버퍼, 바인드 그룹, 메모리 집계 정보를 각각 보관하는 별도의 배치 결과를 만든다. 입력 키가 정확히 같을 때만 결과를 재사용한다. 토큰이 해제된 뒤에도 다른 입력으로 기존 결과를 덮어쓰지 않는다. 호스트의 미제출 명령 버퍼가 예전 GPU 자원을 참조할 수 있기 때문이다.

경로 길이·별 위치 계산도 같은 규칙을 따르며, 직접 지정한 라벨 앵커 역시 변경하지 않는 스냅샷으로 보관한다. paint 단계에서는 시리즈 캐시를 다시 조회하지 않는다. 토큰으로 기록한 명령 버퍼는 같은 `ChartView`를 다시 준비하거나 `refresh_axis` / `update_transform`을 호출하기 전에 제출해야 한다.

winit 루프나 WASM 래퍼처럼 프레임 동안 렌더러를 단독으로 사용하는 호스트는 두 단계를 연속 실행하는 `Renderer::paint(&mut self, …)`를 사용하면 된다. 전체 예제는 [egui_embed.rs](examples/egui_embed.rs)를 참고한다.

### iced 통합 상태

[보관 중인 iced 통합 예제](https://github.com/shim9610/figgy/blob/master/crates/renderer/unsupported/iced_embed_wgpu27.rs)에서 `prepare` / `paint_prepared`의 소유권 패턴을 확인할 수 있다. 다만 iced 0.14는 wgpu 27을 사용하므로 현재 빌드 대상에서는 제외한다. 서로 다른 wgpu 주 버전의 장치·큐·렌더 패스 타입은 공유할 수 없다.

<a id="png-export-메모리-only--저장은-caller"></a>

### 직교 시리즈·매핑·레이어

컬럼 기반 시리즈는 모두 `SeriesConfig::render_type`으로 선언한다([시리즈 선언](CONFIG.md#data_config--시리즈-선언-형식)).
선·산점도·오차 막대·히스토그램·히트맵·등고선 외에 다음을 지원한다.

- **면 채움과 계단선**: 선의 `fill`은 기준선 또는 같은 x 컬럼을 쓰는 다른 시리즈까지 칠하고,
  `interpolation`은 `StepPre`·`StepPost`·`StepMid` 계단 조각을 그린다. 대시와 피킹도 그 경로를
  따른다. [설정](CONFIG.md#면-채움과-계단선)과 `area_step` 예제.
- **점 색·크기 매핑**: 산점도의 점을 값 컬럼으로 색상 막대를 거쳐 색칠하거나, 명시한 범위로
  크기를 정한다(버블 차트). 색과 반지름은 그릴 때마다 셰이더에서 계산하며 점마다 저장하지 않는다.
  [설정](CONFIG.md#point-colour-and-size-maps)과 `value_maps` 예제.
- **벡터장**: `DataRenderType::Vector`는 흩어진 기준점의 (u, v) 컬럼 또는 격자 위 행렬 두 개에서
  표본마다 화살표 하나를 픽셀 또는 데이터 스케일로 그리고, 색상 막대로 색칠할 수 있다.
  [설정](CONFIG.md#vector-fields)과 `vector_field` 예제.
- **보조 Y축·서브플롯·주석**: 오버레이 레이어는 두 번째 Y축에 시리즈를 그리고, 서브플롯은 차트
  여러 개를 연결 축·쌍둥이 축과 함께 격자로 배치하며, 주석 층은 어떤 차트든 기준선·구간·텍스트·
  화살표를 더한다. [오버레이](INTERACTION.md#overlay-layers), [서브플롯](INTERACTION.md#subplots),
  [주석](INTERACTION.md#annotation-layer)과 `overlay_axes`·`subplots` 예제.

### 박스플롯 — 소량 데이터 API

`BoxPlotRenderer`는 미리 집계한 사분위수·중앙값·수염·평균·이상치를 그린다.
가로·세로 및 그룹 배치, 무광·금속 질감, 개별 색·스타일과 선택을 지원한다.
노치는 입력받은 중앙값 신뢰구간으로 그린다. [사용법](BOXPLOT.md)과
`boxplot_editor` 예제를 참고한다. 제목·축·라벨·범례 편집은 [공통 선택·이동 API](INTERACTION.md)를 사용한다. 웹 Studio 연결과 배포는 별도다.

### 레이더 — 소량 데이터 API

`RadarRenderer`는 고정된 범주 집합 위에서 몇 개의 시리즈를 비교한다. 범주마다 스포크 하나,
값 축은 하나를 공유하며, 각 시리즈는 채움·외곽선·꼭짓점 마커가 있는 닫힌 다각형이다. 고리는
다각형(거미줄) 또는 원으로 그린다. 범주·시리즈마다 값 하나를 호스트가 넣으며 스포크별 정규화는
계산하지 않는다. [사용법](RADAR.md)과 `radar_editor` 예제를 참고한다. 제목·라벨·범례 편집은
[공통 선택·이동 API](INTERACTION.md)를 사용한다.

### 극좌표 플롯 — 직교 설정의 좌표계

극좌표 플롯은 `config.polar = Some(PolarOptions { .. })`를 켠 직교 차트다. 같은 선·산점도·면
시리즈, 컬럼 풀, 피킹과 내보내기를 (각도, 반지름)으로 그린다. `bottom_x`가 각도 축, `left_y`가
반지름 축이며 격자는 고리와 스포크, 각도 축은 바깥 고리, 반지름 축은 중심을 축으로 드래그해 돌릴
수 있는 광선이 된다. 각도 단위는 도 또는 라디안, 원은 구멍이 있는 섹터일 수 있고, 세그먼트는 현
또는 호다. 극좌표 형태가 없는 시리즈(히스토그램·히트맵·등고선·오차 막대·벡터)와 스트리밍 소스는
명시적 오류로 거부한다. [사용법](POLAR.md)과 `polar_plot` 예제를 참고한다.

### 범주형 막대 — 소량 데이터 API

`CategoricalRenderer`는 가로·세로 묶음, 누적, 100% 누적 막대를 그린다. 소규모 범주·시리즈·값을
직접 담는 별도 계약이며 `ColumnSource`를 사용하지 않는다. 막대별 색·재질·모서리·외곽선·표기와
선택 상태는 SSOT에서 편집한다. [API 사용법](CATEGORICAL.md)을 참고하거나
`cargo run -p figgy-renderer --example categorical_editor --features egui_demo`로 편집기를 실행한다.
웹 Studio 연결은 별도 작업이다.

### 원형 차트 — 소량 데이터 API

`RadialRenderer`는 파이·도넛·‘기타’ 분할 차트를 그린다. 무광·세라믹·새틴 금속·툰 등의
재질, 둥근 모서리, 외곽선, 호버·선택을 지원한다. 조각별 색·스타일·표기 내용은 SSOT 모델에서
편집한다. 소수의 범주 데이터를 한 번에 받아 그리며 호스트의 wgpu 장치를 공유한다.
[네이티브 편집기](examples/radial_editor.rs)는
`cargo run -p figgy-renderer --example radial_editor --features egui_demo`로 실행한다. [사용법과 예제](RADIAL.md)를 참고한다.
웹 Studio 연결과 배포는 별도다.
문서에 [옵션 기본값과 범위](RADIAL.md#스타일-옵션-기본값과-범위),
[조각별 편집과 상속](RADIAL.md#ssot-편집과-선택),
[호스트 갱신 순서](RADIAL.md#호스트의-갱신-순서), 프레임 수명과 내보내기를 정리했다.

### PNG 내보내기 — 바이트 반환과 파일 저장

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

네이티브에서는 이름에서 `_async`를 뺀 동기 래퍼도 제공한다. 파일 저장은 호출자가 담당한다.
`scale`은 `renderer::MIN_EXPORT_SCALE`(0.25)부터 `renderer::MAX_EXPORT_SCALE`(8.0)까지로 제한한다. DPI에서 배율을 구하려면 표준값 96 DPI를 기준으로 계산하는 `renderer::dpi_to_scale(dpi)`를 사용한다.

폰트, 선, 여백, 격자, 범례의 픽셀 크기가 모두 같은 비율로 커지므로 차트의 비례 관계를 유지하면서 해상도를 높일 수 있다.

---
