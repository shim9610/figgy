# figgy egui examples

figgy charts embedded in egui and eframe hosts. This is a standalone Cargo
workspace with its own `Cargo.lock`, like `tools/playground`. It is not a
workspace member, not a renderer feature, and not part of any published
package, so figgy-renderer itself carries no egui dependency. The examples
use only the renderer's public API.

egui-wgpu is built on one wgpu major version. It must match the one figgy
uses (wgpu 30 for figgy 0.19), so these examples use egui 0.36.

The crate depends on `crates/renderer` by path, so a root dependency bump does
not refresh its `Cargo.lock`. `python3 ci/verify.py static` fails when that
lockfile no longer resolves with `--locked`, and CI's repository job builds the
examples ([repository checks](../../ci/README.md#repository-checks)).

## Run

From the repository root:

```sh
cargo run --locked --manifest-path examples/egui/Cargo.toml --example egui_embed
cargo run --locked --manifest-path examples/egui/Cargo.toml --example radial_editor
cargo run --locked --release --manifest-path examples/egui/Cargo.toml --example ssot_lab
cargo run --locked --release --manifest-path examples/egui/Cargo.toml --example streaming_capacity_demo
cargo run --locked --manifest-path examples/egui/Cargo.toml --example constellation_lab
```

| Example | What it shows |
| --- | --- |
| `egui_embed` | Three panels drawn in egui paint callbacks, with pointer input and PNG export. On Vulkan it saves the renderer's pipeline cache on exit and loads it on the next run (`RendererDevice::with_pipeline_cache`, `pipeline_cache_data`), in a `figgy-egui-example` folder under the system temporary directory |
| `radial_editor` | Selecting, editing and dragging the parts of a radial chart |
| `ssot_lab` | Four charts edited live, one per draw style, sharing one X column |
| `streaming_capacity_demo` | Lazily generated data through `ColumnSource`; add `-- --benchmark` for timings |
| `constellation_lab` | Every `ConstellationOptions` field bound to a slider |

The [example index](../../crates/renderer/examples/README.md#interactive-examples)
describes each one in more detail.
