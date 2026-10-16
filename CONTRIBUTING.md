# Contributing to figgy

Bug reports, small reproductions, documentation corrections and focused pull
requests are welcome. Start with an issue for a new chart family or a public API
change so the data and interaction contracts can be discussed before implementation.

## Report an issue

Use the [issue forms](https://github.com/shim9610/figgy/issues/new/choose). A useful
renderer report includes a minimal Rust example, synthetic input, the package
version or commit, OS, GPU/driver and backend, plus the first error or a before/after
image. State whether the issue occurs during initial preparation, an edit,
streaming, selection, resizing or PNG export. A screenshot is helpful but does not
replace a reproducer.

The WASM wrapper is an integration example. For browser reports, include the
browser version, wrapper revision and host call sequence. The independently
developed figgyplot.com Studio is a separate application; identify which code
reproduces the problem without assuming that either the host or renderer is at fault.

For a suspected vulnerability, follow [SECURITY.md](SECURITY.md) instead of opening
a public issue. Do not upload credentials or confidential datasets.

## Build and test

Clone this repository and use its pinned Rust 1.99.0 toolchain. Rustup installs the
toolchain and `wasm32-unknown-unknown` target from `rust-toolchain.toml`. Python 3.12
or newer is needed for the package checks.

```sh
git clone https://github.com/shim9610/figgy.git
cd figgy
cargo check --locked --workspace --all-targets
cargo test --locked -p figgy-model --all-features
```

The model tests do not require a GPU. Renderer tests need a working graphics
adapter and driver; software adapters are valid test environments when identified
as such. Run the regression relevant to your change, for example:

```sh
cargo test --locked -p figgy-renderer --test bounded_interaction -- --test-threads=1
```

Format with `cargo fmt --all` before committing; CI rejects unformatted code.
For the same formatting, build/documentation and extracted-package checks used
in CI:

```sh
python3 ci/verify.py static
python3 ci/verify.py packages
```

Use `python` where that is the Python 3 executable. Read the
[CI guide](ci/README.md) for platform prerequisites, backend selection, Windows
DXC/WARP setup and diagnostic artifacts. Keep `Cargo.lock` changes intentional;
do not update unrelated dependencies to fix a local setup problem.

## Prepare a pull request

Work on a branch or fork and open a PR against `master`. Keep the change focused,
explain the user-visible behavior, and report which checks ran and any gaps.
New documentation and example explanations should be in English; retain existing
Korean documentation. Public API or behavior changes need corresponding guide,
example and changelog updates. The maintainer coordinates release version bumps.

For chart changes, verify the affected parts of the existing workflow:

- Initial rendering and subsequent model edits, including required cache updates
  and reuse when the data or geometry has not changed.
- Picking, selection, dragging of editable elements and plot-area resize handles.
- Hover/selection behavior and PNG export at the relevant scale.
- Serialization round trips when changing serializable model fields.

State which items are not applicable and why. Small-data radial, categorical and
boxplot charts use bounded models; adding them does not require streaming support.
Regression tests should exercise the renderer's public APIs. Distinguish renderer
tests from host event wiring and deployed application checks.

Run the relevant shader consistency test if you change shared WGSL definitions;
the common declarations are documented in
[SHADER_COMMON.md](crates/renderer/src/data_render/SHADER_COMMON.md).

Public, non-draft PRs run the three-OS package/rendering matrix and CodeQL. There
are no push or scheduled triggers for those workflows. Draft PRs wait until
marked ready for review. Compiler and rustdoc warnings fail the package checks.
CI adapters are software or virtualized; macOS/Metal remains experimental.

## License

Unless explicitly stated otherwise, contributions are provided under the
repository's [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) licenses. Preserve
third-party notices, including the separate bundled-font licenses, and only
contribute material you have the right to share.
