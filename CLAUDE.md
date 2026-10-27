# Agent guide for figgy

Rules for coding agents working in this repository. They come from the
maintainer and override an agent's own defaults. When a plan or change would
break one of them, stop and ask instead of working around it.

## Design rules

1. **The host computes data; the renderer computes appearance.** The
   renderer draws the data it is given and never produces new values the
   user reads as data: no binning, no density estimates, no integration (such
   as streamlines), no running totals, no statistics shown as data, and no
   per-axis normalization of the data. A gradient or magnitude shown as data,
   such as arrows of ∇f or a color by |v|, is host work. Hosts supply data in
   the dimensions the chart needs. Values that only decide how the given data
   looks are the renderer's job: color mapping, antialiasing, shading normals
   and shadows, projection (including normalizing ternary components only to
   place them), dash phase, arrowhead direction along a drawn path, jitter,
   and display parameters chosen from column statistics, for example
   `auto_fit_*`, the tick plan or a size-domain fit. Per-point appearance
   values are computed in the shader on every draw and never stored (rule 2).
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
10. **Column-backed series have no count limits.** A series whose data
    comes through `ColumnSource` draws every registered value. There is no cap
    on rows, groups, categories, series in a group, members, labels, slots or
    any other count, and no fixed-size array, uniform or loop bound sized by
    one. The only failure allowed is at column registration, when the memory
    cannot be allocated or the device cannot hold the buffer. A cost that
    grows with the data (vertices, pick work) is the host's to budget: the
    renderer measures it in tests and documents it, and never caps it. A
    DataLess chart (defined by the SSOT alone, no `ColumnSource`, such as the
    radial chart) may have a limit only with the maintainer's approval in the
    issue before it is written. Limits on things that are not data counts
    (text length, nesting depth, tick counts) are not covered by this rule,
    and a new one still needs the plan to name it (#107).

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
  CPU, the renderer produces no value the user reads as data (rule 1:
  appearance and display parameters only), and new per-point input enters
  through `ColumnSource`.
- **Limits (rule 10).** The diff adds no count limit on a column-backed
  series: no `MAX_*` check, fixed-size array, uniform array or loop bound
  sized by a count of data, categories, series or labels, and no cap that
  fails or stops drawing after registration. Each test that adds a
  column-backed feature registers past any size the code once capped and
  draws. A limit on a DataLess chart cites the issue that approved it.
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
