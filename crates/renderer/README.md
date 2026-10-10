# figgy-renderer

A native Rust chart renderer: wgpu draws data, while tiny-skia and swash rasterize
axes and text. Embed it in an existing wgpu application or use its window and
headless export helpers. No GUI framework is required by the default features.

The Cargo package is **`figgy-renderer`**; the Rust library name remains
**`renderer`** to preserve existing imports. This source release has not yet
been published to crates.io. For a source checkout:

```toml
[dependencies]
renderer = { package = "figgy-renderer", path = "../figgy/crates/renderer" }
wgpu = "30"
```

For an existing Git dependency, retain its URL/revision and dependency key, and
add `package = "figgy-renderer"`. The selected revision must contain that package
name. Older public revisions still use the old package name.

- `Renderer`: column-based scientific charts, fields and replayable streaming.
- `RadialRenderer`: bounded pie/donut charts and procedural materials.
- `CategoricalRenderer`: bounded individual, grouped and stacked bars.
- `BoxPlotRenderer`: supplied statistical summaries, including grouped/notched boxes.

These renderers share a host-provided `RendererDevice`. Their data models and
GPU budgets are separate. Small charts do not require column sources or streaming.

Rust **1.99** is the declared support floor. Direct GPU sharing requires **wgpu 30**.
Lower Rust versions have not been validated. The optional `serde` feature enables
serialization of re-exported model types without a separate model dependency;
it is not a project file format. `egui_demo` enables the native egui examples.
WASM compilation is also supported as a consumer example; the web wrapper is not
part of this Rust registry package.

**macOS / Metal support is experimental, not stable.** Coordinate precision,
picking and render-path comparison regressions are still being investigated.
The source repository contains a separate, opt-in precompiled Metal shader probe;
it is not an enabled renderer feature or a completed precision fix. See the
[backend status](https://github.com/shim9610/figgy/blob/master/ci/STATUS.md).

The crate-level API documentation covers ownership, frame validity, target
constraints, source replay and compatibility. Individual chart modules contain their usage
guides. The [source repository](https://github.com/shim9610/figgy) contains the
gallery and full native integration examples.

Code is licensed under MIT OR Apache-2.0. Bundled fonts retain their respective
OFL notices in `fonts/`.

Native package tests run with `cargo test --features serde`. They include GPU
pixel, cache and memory regressions and require a working adapter and device;
initialization errors fail the tests. Software Vulkan can be used on machines
without a GPU. No window is required. Large stress/probe tests marked `#[ignore]`
are opt-in. Tests and their fixtures are included in the package.

The public repository runs these checks on standard Linux/Vulkan, Windows/DX12
and macOS/Metal runners. See the [CI guide](https://github.com/shim9610/figgy/blob/master/ci/README.md)
for commands, artifacts and the limits of software/virtual GPU coverage.
