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

macOS / Metal support is **experimental and not yet stable**. Known failures
include large-coordinate precision, picking at endpoint ties, and differences
between resident and streamed composition. A separate precompiled Metal shader
experiment does not change the default renderer or establish a precision guarantee.
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

## Ownership and frame validity

`Renderer::prewarm_all` initializes the optional Cartesian pipelines before first
use. Calling it again on the same renderer reuses them without creating new shader
modules or pipelines. It does not initialize separate radial/categorical/boxplot
renderers. WASM prewarm uses the same explicit descriptors as the production
constructors; it does not depend on auto-layout guesses to warm a different layout.

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

Each renderer accounts for its own resources; shared-device applications must
combine those budgets and their own target allocations. These counters are not
total process memory or driver VRAM measurements. Static limits and budget checks
do not guarantee that an external driver allocation will succeed.

## Render targets, coordinates and output

Small-chart frames require a target with the prepared physical size, the renderer's
format and sample count 1. `draw` sets viewport/scissor to that target. For a host
UI subregion or a multisampled host pass, draw to a matching offscreen texture and
composite it. Supported formats are RGBA8/BGRA8, linear or sRGB.

Small-chart `prepare` takes logical size and scale; picking takes logical
coordinates. Frame sizes and exposed geometry rectangles are physical pixels.
Prepare at the actual output scale to rasterize labels at that resolution.
Each module documents its accepted size/scale bounds; the Cartesian export scale
contract differs from the small-chart contract.

RGBA export returns pixels; PNG encoding returns bytes. The caller decides where
to send or store them. Native blocking helpers and async variants are both
available. No project file persistence or application event loop is required.

## Compatibility policy

Changing package identity requires a Cargo dependency declaration update; it does
not rename Rust items. This release adds the bounded chart modules while retaining existing Cartesian
function signatures, model fields and enum variants. Public low-level modules remain available; they must not
be removed silently during packaging.

The public Rust surface includes struct literals and exhaustive enums. Removing
or changing fields, adding required fields or enum variants, changing trait
requirements, and adding `non_exhaustive` to existing types can break consumers.
Incompatible changes require a new 0.x minor version and migration notes; patch
updates must preserve the supported contract. A new wgpu major needs the same
review. The source versions are figgy-renderer 0.13.1 and figgy-model 0.8.0. They have not
been published to crates.io; publishing a source commit does not publish a registry package.

The optional serde representation is not an application file-format version.
Same-version round-trip tests do not promise arbitrary cross-version migrations.
