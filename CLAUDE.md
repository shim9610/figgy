# Agent guide for figgy

Rules for coding agents working in this repository. They come from the
maintainer and override an agent's own defaults. When a plan or change would
break one of them, stop and ask instead of working around it.

## Design rules

1. **The renderer draws the data it is given.** It never changes the dimension
   of the data or computes statistics from it: no binning, no density
   estimates, no gradients from a scalar field, no magnitudes from vectors, no
   per-axis normalization. Hosts supply data in the dimensions the chart needs.
   Visual helpers that only choose display parameters from column statistics
   are allowed, for example `auto_fit_*`, `auto_ticks` or a size-domain fit.
2. **No data copies.** Copying is allowed only for SSOT configuration. A
   resident column's only copy is the GPU column pool; the CPU keeps scalar
   statistics only (`column_pool.rs`). Values derived per point, such as mapped
   colors or radii, are computed in the shader on every draw and never stored.
3. **No `Vec` that the plan does not name.** Prefer fixed-size data, borrowed
   slices or writing directly into mapped GPU memory.
4. **No lock-based synchronization.** No `Mutex`, `RwLock` or similar.
5. **Per-point data enters only through `ColumnSource`.** Do not invent new data
   types or containers for incoming data. If a new kind of data is needed (for
   example vector components), extend `ColumnSource` and the pool's column
   types.
6. **New charts and features implement every existing interaction.** Nothing is
   left out or deferred: hover, click selection, `step_selection`, the picked
   highlight, decoration and plot-area drag/resize, the annotation layer, PNG
   export at 1x/2x, serialization round trips, SSOT invalidation, overlay
   layers and subplots where the chart family supports them, and the WASM
   wrappers.
7. **No unplanned code paths.** Reuse the existing pattern for the job. A new
   path, binding, cache or module needs the maintainer's approval in the issue
   before it is written.
8. **Streaming is not extended.** Existing streaming support stays as it is.
   New features reject streamed sources with an explicit error.
9. **Vector fields use a layered shader assembled in Rust** (glyph, input,
   projection and entry fragments), so a later 3D projection can reuse the
   glyph (#25).

## Before calling a change done

Check the diff against the design rules item by item and report the result in
the PR's "Chart behavior" section. A missing item is a defect to fix, not a
follow-up.

- **Interactions (rule 6).** For every new chart, feature or drawn element,
  confirm each interaction is implemented and tested, or state why it does not
  apply: hover, click selection, `step_selection`, the picked highlight,
  decoration drag (title, legend, axes, tick labels, axis titles), plot-area
  drag and resize, the annotation layer, PNG export at 1x and 2x,
  serialization round trips, SSOT invalidation, overlay layers and subplots,
  and the WASM wrappers. An element a rectangular chart lets the user select
  or drag must be selectable or draggable in the new chart too.
- **Locks (rule 4).** The diff adds no `Mutex`, `RwLock` or other lock-based
  synchronization.
- **Allocations (rule 3).** Every new `Vec`, `vec!` or `collect` outside tests
  is one the plan names; list them in the PR.
- **Copies (rules 1, 2 and 5).** No per-point data is stored or copied on the
  CPU, nothing is computed from the data beyond display parameters, and new
  per-point input enters through `ColumnSource`.
- **Streaming and new paths (rules 7 and 8).** Streamed sources are rejected
  with an explicit error, and every new path, binding, cache or module has the
  maintainer's approval in the issue.

## Working rules

- Everything committed is in English: code, comments, docs, commit messages,
  issues and PRs. Keep existing Korean documentation.
- Follow `CONTRIBUTING.md`: `cargo fmt --all`, `python3 ci/verify.py static`,
  and the PR template in `.github/PULL_REQUEST_TEMPLATE.md`.
- Shared shader code lives in `crates/renderer/src/data_render/common/`, one
  fragment per block, spliced into each module by `wgsl_module!`
  (`shader_assembly.rs`); `SHADER_COMMON.md` documents the contracts. A CPU
  twin of a shader function is test support, never product code.
- New serialized fields are additive: `Option` or defaulted, with
  `skip_serializing_if`, so older documents parse and re-serialize
  byte-identically. Breaking Rust literals get a CHANGELOG migration note.
- Check external facts at the source. Publication state comes from the
  crates.io index, not from wording in the docs.
- Do not put model identifiers in code or docs. A commit trailer or PR
  footer naming the model that wrote it is fine.
