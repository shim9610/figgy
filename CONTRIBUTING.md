# Contributing to figgy

Bug reports, small reproductions, documentation corrections and focused pull
requests are welcome. Start with an issue for a new chart family or a public API
change so the data and interaction contracts can be discussed before implementation.

## Report an issue

Use the [issue forms](https://github.com/shim9610/figgy/issues/new/choose). A useful
renderer report includes a minimal Rust example, synthetic input, the crates.io
version or, for a Git checkout, the commit hash (master can be ahead of the
published release while carrying the same version number), OS, GPU/driver and
backend, plus the first error or a before/after image. State whether the issue occurs during initial preparation, an edit,
streaming, selection, resizing or PNG export. A screenshot is helpful but does not
replace a reproducer.

The WASM wrapper in `crates/web` is figgy's browser product. For browser reports, include the
browser version, wrapper revision and host call sequence. The independently
developed figgyplot.com Studio is a separate application; identify which code
reproduces the problem without assuming that either the host or renderer is at fault.

For a suspected vulnerability, follow [SECURITY.md](SECURITY.md) instead of opening
a public issue. Do not upload credentials or confidential datasets.

## Build and test

Clone this repository and use its pinned Rust 1.99.0 toolchain. Rustup installs the
toolchain and `wasm32-unknown-unknown` target from `rust-toolchain.toml`. Python 3.12
or newer is needed for the package checks. Before installing the hook below,
check `git config --get core.hooksPath` and existing `.git/hooks`. Preserve any
custom hooks and chain this guard instead of replacing their configuration.

```sh
git clone https://github.com/shim9610/figgy.git
cd figgy
git config --local core.hooksPath .githooks
cargo check --locked --workspace --all-targets
cargo test --locked -p figgy-model --all-features
```

The tracked pre-push hook checks the exact commits being pushed, including a
branch other than the current checkout. It checks links against Cargo's actual
package file lists offline, without building or uploading. A repository-relative
link to a sibling crate can work on GitHub but break after registry extraction;
use a public source URL for those links. Run `python3 -B ci/package_docs.py` to
check uncommitted documentation before committing.

Hook installation is local to each clone. A custom hook must forward Git's
original stdin to `.githooks/pre-push` when chaining it. CI runs the same package-link
guard early even when a local hook is bypassed.

The model tests do not require a GPU. Renderer tests need a working graphics
adapter and driver; software adapters are valid test environments when identified
as such. Run the regression relevant to your change, for example:

```sh
cargo test --locked -p figgy-renderer --features serde --test bounded_interaction -- --test-threads=1
```

`--features serde` adds the serialization round trips that CI also runs.

Format with `cargo fmt --all` before committing; CI rejects unformatted code.
The formatting, build/documentation and extracted-package checks used in CI run
with:

```sh
python3 ci/verify.py static
python3 ci/verify.py packages
```

CI also sets `RUSTFLAGS=-D warnings` and `RUSTDOCFLAGS=-D warnings`, so compiler
and rustdoc warnings fail the package checks. Set both variables to reproduce
that locally.

Use `python` where that is the Python 3 executable. Read the
[CI guide](ci/README.md) for platform prerequisites, backend selection, Windows
DXC/WARP setup and diagnostic artifacts. Keep `Cargo.lock` changes intentional;
do not update unrelated dependencies to fix a local setup problem.

## Prepare a pull request

Work on a branch or fork, open a PR against `master` and fill in each section of
the [pull request template](.github/PULL_REQUEST_TEMPLATE.md). Keep the change focused,
explain the user-visible behavior, and report which checks ran and any gaps.
New documentation and example explanations should be in English; retain existing
Korean documentation. Public API or behavior changes need corresponding guide,
example and changelog updates. The maintainer coordinates release version bumps.

A new chart or feature implements and tests every existing interaction below.
For other chart changes, verify the items the change affects:

- Initial rendering and subsequent model edits, including cache invalidation
  and reuse when the data or geometry has not changed.
- Hover, click selection, selection stepping (`step_selection`) and the picked
  highlight.
- Dragging the chart title, legend, colorbar, axes, tick labels and axis titles,
  plus plot-area drag and resize handles.
- The annotation layer, and overlay layers and subplots where the chart family
  supports them.
- PNG export at 1x and 2x.
- Serialization round trips when changing serializable model fields.
- The WASM wrappers in `crates/web`.

State which items are not applicable and why. An element that a rectangular chart
lets the user select or drag is selectable or draggable in a new chart as well.

Radial charts use a bounded small-data model and do not need streaming
support; categorical bars, box plots and radar series are column series that
reject streamed sources. Existing streaming support is not extended: a new chart
or feature rejects streamed sources with an explicit error, as polar charts, vector
series, line fills and step lines do with `FiggyError::InvalidSeriesConfig`.

Regression tests should exercise the renderer's public APIs. Distinguish renderer
tests from host event wiring and deployed application checks.

Shared WGSL code lives in `crates/renderer/src/data_render/common/`, one fragment
per block, which `wgsl_module!` (`crates/renderer/src/shader_assembly.rs`) splices into every module
that uses it at compile time. To change it, edit the fragment, then the Rust
struct or constant that mirrors a changed layout, and the contract in
[SHADER_COMMON.md](crates/renderer/src/data_render/SHADER_COMMON.md) it
implements; follow that guide's steps and run
`cargo test --locked -p figgy-renderer --test shader_consistency`.

Public, non-draft PRs run the three-OS package/rendering matrix and CodeQL. There
are no push or scheduled triggers for those workflows. Draft PRs wait until
marked ready for review. CI adapters are software or virtualized; macOS/Metal
remains experimental.

## License

Unless explicitly stated otherwise, contributions are provided under the
repository's [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) licenses. Preserve
third-party notices, including the separate bundled-font licenses, and only
contribute material you have the right to share.
