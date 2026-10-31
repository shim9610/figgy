# figgy-renderer

A native Rust chart renderer for scientific plots, styled dashboards and small
data charts. Draw lines, scatter plots, error bars, histograms, heatmaps, contours,
vector fields, polar and ternary plots, plus pies, donuts, categorical bars, boxplots and
radar charts; edit their models and export PNGs.
Embed charts in an existing wgpu application or render without a window.
The default library has no GUI framework dependency.

[Quick start and gallery](https://github.com/shim9610/figgy#quick-start) ·
[Runnable chart examples](https://github.com/shim9610/figgy/tree/master/crates/renderer/examples) ·
[Native usage](https://github.com/shim9610/figgy/blob/master/crates/renderer/USAGE.md) ·
[API contracts](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md) ·
[Configuration](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md) ·
[Architecture](https://github.com/shim9610/figgy/blob/master/crates/renderer/ARCHITECTURE.md).

## Install and run

Use Rust **1.99 or later** and a working graphics adapter/driver.
Add these dependencies to your application's `Cargo.toml`:

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.18.1" }
wgpu = "30"
pollster = "1"
```

The Cargo package is **`figgy-renderer`**; the Rust import is **`renderer`**.
The `version` line installs figgy-renderer 0.18.1, with figgy-model 0.12.1, from
[crates.io](https://crates.io/crates/figgy-renderer). For a local checkout,
replace `version` with `path = "../figgy/crates/renderer"`. Code written for
0.17.0 needs the changes in the
[migration notes](https://github.com/shim9610/figgy/blob/master/CHANGELOG.md#migrating-from-0170).

Copy the complete [line example](https://github.com/shim9610/figgy/blob/master/crates/renderer/examples/line.rs)
into `src/main.rs` and run `cargo run`. It creates a device, uploads two columns,
draws axes and a sine wave, and writes a 960 × 600 `line.png` without opening a window.
The crate root page of the API documentation shows the example after this
README. From a source checkout, run
`cargo run --locked -p figgy-renderer --example line -- line.png`.

The application needs its own `wgpu` and `pollster` dependencies for this example.
Hosts with an async executor can await export directly instead of using pollster.
Hosts that already own a matching wgpu 30 device/queue can share them through
`RendererDevice`. They do not need a second device or a particular GUI toolkit.

## Choose a renderer

| Charts | Entry point | Input and guide |
| --- | --- | --- |
| Lines, scatter, error bars, histograms, heatmaps, contours | `Renderer` | Numeric columns and `SeriesConfig`; [native usage](https://github.com/shim9610/figgy/blob/master/crates/renderer/USAGE.md) |
| Hexbins: host-binned counts on a hexagonal grid | `Renderer` | A `Heatmap` whose `MatrixRef.grid_layout` is `Hexagonal`, shaded flat on linear axes; [hexagonal grids](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#hexagonal-grids-hexbins) |
| Colour images (imshow) | `Renderer` | An `RgbImage` series: three channel grids on one lattice mapped from a `range`, no colourbar; [colour images](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#colour-images-imshow) |
| Pies and donuts, including 3D/material styles | `RadialRenderer` | `RadialChart` with its slice values in the model; [radial guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/RADIAL.md) |
| Individual, grouped and stacked bars | `Renderer` | `CategoryBars` series over a categorical axis, or a legacy `CategoricalChart` read by `Config::from_categorical`; [bar guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/CATEGORICAL.md); on a polar chart the bars fan around the centre as a [rose](https://github.com/shim9610/figgy/blob/master/crates/renderer/POLAR.md#roses-polar-bars) |
| Grouped/notched boxes and outliers | `Renderer` | `BoxPlot` series over a categorical axis, reading a record of supplied statistics and a ragged outlier column, or a legacy `BoxPlotChart` read by `Config::from_boxplot`; [boxplot guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/BOXPLOT.md) |
| Radar (spider) charts | `Renderer` with `Config.polar` | `Radar` series over a categorical angle axis, one value column per series, or a legacy `RadarChart` read by `Config::from_radar`; [radar guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/RADAR.md) |
| Polar plots: lines, scatter and areas in (angle, radius) | `Renderer` with `Config.polar` | The same columns as rectangular charts with scatter, line and scatter-line `SeriesConfig`; histograms, heatmaps, contours, error bars, vectors, overlay layers and streamed sources are rejected; [polar guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/POLAR.md) |
| Ternary plots: markers, lines and region fills of three-component samples | `Renderer` with `Config.ternary` | Three columns per `Ternary` series (`x_column`, `y_column`, `c_column`), unnormalized; a `region` fills the polygons the samples outline (even-odd); line fills, step lines, overlay layers and streamed sources are rejected; [ternary guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/TERNARY.md) |
| 3D surfaces: a heatmap's grid as a lit surface, contours on the floor, points and trajectories (a scatter or line series with a `z_column`) | `Renderer` with `Config.view_3d` | The heatmap's own columns; overlays, line offsets and streamed sources are rejected; [surface guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/SURFACE.md) |

The renderer re-exports model types: most applications do not need a separate
`figgy-model` dependency. Pies and donuts are the
DataLess chart: it draws one validated model with at most 64 slices,
takes no column sources or streamed data, and computes no
statistics from raw samples. Categorical bars and box plots are `Renderer`
series over a categorical axis, and radar series over the categorical angle
axis of a polar chart; the legacy `CategoricalChart`, `BoxPlotChart` and
`RadarChart` documents are deprecated readers for this release, removed in the
next milestone. Both renderers can share a
`RendererDevice`; each maintains its own resource budget. Vector fields, point
colour and size maps, area fills, step lines, overlay axes and subplots are
`Renderer` features declared in `SeriesConfig` and `Config`; see the
[configuration reference](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md). Budgets apply to checked
allocation paths, not total device VRAM; see the
[accounting scope](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md#gpu-memory-accounting-and-budget-scope).

## Edit, pick and export

Prepare and draw the current model, pick against the displayed frame, then edit
the model. For Cartesian charts, refresh changed axes/decorations and recreate
affected caller-owned `ChartStyle` values before preparing again. The radial
renderer updates its derived resources in `prepare`.
Export that same model to preserve selection, moved
labels and the drawing area. Data picking and decoration hit maps are separate;
use the renderer's measured bounds when selecting titles, labels, axes or handles.
Reference lines, shaded spans, text boxes and arrows live in each chart's
`annotations` layer, which has its own hit map and cache: spans are drawn below
the data and the other items above every decoration; see the
[interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer).

The [API workflow](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md#native-edit-pick-and-export-workflow)
includes Cartesian refresh and radial edit/export examples, and explains frame lifetime and coordinate
units. The [interaction contracts](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)
cover shared selection, dragging and area handles (in Korean) and titles,
layers, subplots and annotations (in English). Runnable editors are
`winit_simple` and `radial_editor`; the second requires `--features egui_demo`.

Automatic Cartesian streaming fits the axes only on request: call
`Renderer::request_stream_auto_fit(chart_id, padding)` when the initial range is
unknown or the user asks for a fit, not on every chunk, repaint or tab switch.
Treat the image as exact only when `Renderer::stream_status` reports
`StreamingState::Complete`; `AllSubmitted` means every chunk was submitted, not
that the final image is drawn. A registered stream source must be replayable:
it returns the same encoded values for the same source revision; see the
[input contracts](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md#entry-points-and-input-contracts).

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
OFL and GUST notices in `fonts/`: Liberation Sans and Comic Neue under the SIL
OFL 1.1, and New Computer Modern Math, the math face, under the GUST Font
License (shipped unmodified). The package includes source, shaders, fonts, guides,
runnable examples, tests and test fixtures. Gallery PNG/GIF assets and the unsupported
iced integration source stay in the Git repository and are not installed with
the crate. Architecture illustrations link to a fixed public source revision.

Native package tests run with `cargo test --features serde`. They include GPU
pixel, cache and memory regressions and require a working adapter and device;
initialization errors fail the tests. Software Vulkan can be used on machines
without a GPU. No window is required. Large stress/probe tests marked `#[ignore]`
are opt-in.

CI runs these tests on the platforms listed above; see the
[CI guide](https://github.com/shim9610/figgy/blob/master/ci/README.md) for the
commands and artifacts.
