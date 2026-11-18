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

## Monotonic-x line series: rules M1–M7

A separate rule set for the monotonic-x line series (#271): a
`DataRenderType::MonotonicLine` series fed through `MonotonicSource`. It
applies to that source only; rules 1, 2, 5 and 8 keep governing every other
series. `crates/model/MONOTONIC.md` describes the core and the renderer side.

- **M1. Input.** Data enters only through `MonotonicSource`, which carries
  the values, `start` and `interval`, lent to every call that reads it (the
  monotonic chart's `set_monotonic`, `append_monotonic` with the new
  samples, `resize`, `fit_layout`, an export at another scale) and never
  retained. No call takes `start` or `interval` separately. No other
  container, and no `ColumnSource` upload of the samples.
- **M2. Reduction is appearance.** The renderer may reduce to M4 per pixel
  column. M2 is stated at display resolution: after a full call or a
  whole-column change each column covers the span drawing every segment
  covers; while sliding by fractions of a column the image differs by less
  than one pixel column, and the next full call realigns it exactly; a NaN
  gap narrower than a column is drawn joined. A test compares the envelope
  draw with the full draw at 1x and 2x.
- **M3. Cache bound.** The CPU cache holds only the M4 envelope (4 values
  per pixel column and channel, sized by the plot width and the channel
  count, never by the data length) and the scalars it was built with. Never
  a copy of the data.
- **M4. Recompute.** The full call reduces from scratch every time. Only
  `append_monotonic` merges new samples, and only a sliding append drops
  columns from the front. The scale never changes inside an append. A chart
  with monotonic series is a monotonic chart (`register_monotonic_chart`),
  reached only through its `MonotonicChartId`: no general chart call takes
  it, and the general calls refuse a monotonic line series. Only four calls
  on its handle move the x-to-pixel mapping, each taking the source:
  `set_monotonic` (the only x range or autoscale), `append_monotonic`,
  `resize` (chart area, plot area, display; atomic) and `fit_layout`. Its
  document editor (`MonotonicConfig`) has no mapping setter and refuses an
  edit that would move it. The margin beyond the plot's edges is a fixed 64
  pixel columns, so a stroke width is a style edit (strokes wider than about
  120 device px may miss edge ink). Draws and picks never fail for a missing
  source.
- **M5. Not streaming.** This source does not use, extend or depend on the
  streaming runtime (rule 8). A streamed source is not a `MonotonicSource`.
- **M6. Interactions.** Rule 6 applies in full. Picks, the picked highlight
  and `step_selection` work on the kept points and report logical sample
  indices, without the source.
- **M7. No count limits.** Rule 10 applies: no cap on samples, channels,
  series, columns or stride.

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
  issues and PRs. Keep the Korean sections of the guides. `README.ko.md` stays
  a short Korean entry page (introduction, install, quick start and links);
  it does not mirror the English README's gallery, chart table or example
  list, which live in `README.md` only.
- Follow `CONTRIBUTING.md`: `cargo fmt --all`, `python3 ci/verify.py static`,
  and the PR template in `.github/PULL_REQUEST_TEMPLATE.md`.
- Shared shader code lives in `crates/renderer/src/data_render/common/`, one
  fragment per block, spliced into each module by `wgsl_module!`
  (`shader_assembly.rs`); `SHADER_COMMON.md` documents the contracts. A CPU
  twin of a shader function is test support, never product code.
- Older documents keep working. A document written by an earlier release
  parses, reads back as the same model value after a save, and exports the
  same image (PNG at 1x and 2x) before and after the save
  (`tests/released_documents.rs`, `tests/released_documents_render.rs`). Its
  JSON may be written in a newer form, for example where a fact stored
  twice gets one owner; once saved, it re-serializes unchanged. New
  serialized fields are `Option` or defaulted, with `skip_serializing_if`. A
  changed wire form and breaking Rust literals get a CHANGELOG migration
  note.
- Check external facts at the source. Publication state comes from the
  crates.io index, not from wording in the docs.
- Telling objects apart and detecting stale tokens reuse one of the patterns
  in "Identity and staleness checks" in `crates/renderer/ARCHITECTURE.md`
  (issued number, allocation identity, shared storage, generation). A new
  pattern needs the maintainer's approval in the issue (rule 7).
- Do not put model identifiers in code or docs. A commit trailer or PR
  footer naming the model that wrote it is fine.
