# figgy-renderer

A native Rust chart renderer for scientific plots, styled dashboards and small
data charts. Draw lines, scatter plots, error bars, histograms, heatmaps, contours,
pies, donuts, categorical bars, boxplots and radar charts; edit their models and
export PNGs.
Embed charts in an existing wgpu application or render without a window.
The default library has no GUI framework dependency.

[Quick start and gallery](https://github.com/shim9610/figgy#quick-start) ·
[Runnable chart examples](https://github.com/shim9610/figgy/tree/master/crates/renderer/examples) ·
[Native usage](https://github.com/shim9610/figgy/blob/master/crates/renderer/USAGE.md) ·
[API contracts](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md) ·
[Configuration](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md) ·
[Architecture](https://github.com/shim9610/figgy/blob/master/crates/renderer/ARCHITECTURE.md).

## Install and run

Use Rust **1.99 or later** and a working graphics adapter/driver. This source
targets **0.17.0**, with model **0.11.0**. The dependency below installs this
release from [crates.io](https://crates.io/crates/figgy-renderer) once available;
use the local path below for a checkout before publication.
Add these dependencies to your application's `Cargo.toml`:

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.17.0" }
wgpu = "30"
pollster = "1"
```

The Cargo package is **`figgy-renderer`**; the Rust import is **`renderer`**.
For a local checkout, replace `version` with `path = "../figgy/crates/renderer"`.

Copy the complete [line example](https://github.com/shim9610/figgy/blob/master/crates/renderer/examples/line.rs)
into `src/main.rs` and run `cargo run`. It creates a device, uploads two columns,
draws axes and a sine wave, and writes a 960 × 600 `line.png` without opening a window.
It is also included below on the generated API landing page. From a source
checkout, run `cargo run --locked -p figgy-renderer --example line -- line.png`.

The application needs its own `wgpu` and `pollster` dependencies for this example.
Hosts with an async executor can await export directly instead of using pollster.
Hosts that already own a matching wgpu 30 device/queue can share them through
`RendererDevice`. They do not need a second device or a particular GUI toolkit.

## Choose a renderer

| Charts | Entry point | Input and guide |
| --- | --- | --- |
| Lines, scatter, error bars, histograms, heatmaps, contours | `Renderer` | Numeric columns and `SeriesConfig`; [native usage](https://github.com/shim9610/figgy/blob/master/crates/renderer/USAGE.md) |
| Pies and donuts, including 3D/material styles | `RadialRenderer` | `RadialChart` with bounded slice values; [radial guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/RADIAL.md) |
| Individual, grouped and stacked bars | `CategoricalRenderer` | `CategoricalChart` with category/series IDs; [bar guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/CATEGORICAL.md) |
| Grouped/notched boxes and outliers | `BoxPlotRenderer` | `BoxPlotChart` with supplied statistics; [boxplot guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/BOXPLOT.md) |

The renderer re-exports model types: most applications do not need a separate
`figgy-model` dependency. Bounded charts do not use column sources or streaming
and do not compute statistics from raw samples. All four renderers can share a
`RendererDevice`; each maintains its own resource budget. Budgets apply to checked
allocation paths, not total device VRAM; see the
[accounting scope](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md#gpu-memory-accounting-and-budget-scope).

## Edit, pick and export

Prepare and draw the current model, pick against the displayed frame, then edit
the model. For Cartesian charts, refresh changed axes/decorations and recreate
affected caller-owned `ChartStyle` values before preparing again. Bounded chart
renderers update their derived resources in `prepare`.
Export that same model to preserve selection, moved
labels and the drawing area. Data picking and decoration hit maps are separate;
use the renderer's measured bounds when selecting titles, labels, axes or handles.
Reference lines, shaded spans, text boxes and arrows live in the chart's
annotation layer, drawn by one pass around every family's frame with its own
hit map and cache; see the [interaction guide](INTERACTION.md#annotation-layer).

The [API workflow](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md#native-edit-pick-and-export-workflow)
includes Cartesian refresh and bounded edit/export examples, and explains frame lifetime and coordinate
units. [Interaction contracts](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)
cover shared selection, dragging and area handles. Runnable editors are
`winit_simple`, `radial_editor`, `categorical_editor`, `boxplot_editor` and
`radar_editor`; all but the first require `--features egui_demo`.

Replayable Cartesian streams use an explicit fit request when the initial range
is unknown or the user requests fit. Do not request fit on every chunk, repaint
or tab switch. Wait for final completion before treating the image as exact.

## Features and platform support

| Feature | Purpose |
| --- | --- |
| Default (no optional features) | Native renderer, headless output and host integration |
| `serde` | Serialization of re-exported model types; no application file format |
| `egui_demo` | Optional dependencies for native GUI examples |

Direct GPU sharing requires **wgpu 30**. CI covers native Linux/Vulkan,
Windows/DX12 and macOS/Metal, plus WASM compilation. Software Vulkan/WARP and
virtualized Metal results do not certify every physical GPU driver. Lower Rust
versions have not been validated. WASM is an integration path; the separate web
wrapper and figgyplot.com Studio are not part of this Rust registry package.

**macOS / Metal support is experimental, not stable.** Coordinate precision,
picking and render-path comparison regressions are still being investigated.
The source repository contains a separate, opt-in precompiled Metal shader probe;
it is not an enabled renderer feature or a completed precision fix. See the
[backend status](https://github.com/shim9610/figgy/blob/master/ci/STATUS.md).

API documentation builds without a GPU. docs.rs is configured for native Linux
and WASM with `serde`; GUI example dependencies are not needed for those builds.
To browse locally, run `cargo doc --locked -p figgy-renderer --features serde --no-deps --open`.
The [source repository](https://github.com/shim9610/figgy) contains the gallery and
full native integration examples.

## License and verification

Code is licensed under MIT OR Apache-2.0. Bundled fonts retain their respective
OFL notices in `fonts/`. The package includes source, shaders, fonts, guides,
runnable examples and test fixtures. Gallery PNG/GIF assets and the unsupported
iced integration source stay in the Git repository and are not installed with
the crate. Architecture illustrations link to a fixed public source revision.

Native package tests run with `cargo test --features serde`. They include GPU
pixel, cache and memory regressions and require a working adapter and device;
initialization errors fail the tests. Software Vulkan can be used on machines
without a GPU. No window is required. Large stress/probe tests marked `#[ignore]`
are opt-in. Tests and their fixtures are included in the package.

The public repository runs these checks on standard Linux/Vulkan, Windows/DX12
and macOS/Metal runners. See the [CI guide](https://github.com/shim9610/figgy/blob/master/ci/README.md)
for commands, artifacts and the limits of software/virtual GPU coverage.
