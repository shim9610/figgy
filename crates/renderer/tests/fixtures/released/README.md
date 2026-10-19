# Documents written by released versions

Each `model-<version>-<name>.json` file holds the exact bytes that
`serde_json::to_string` returned for one document built with the published
figgy-model of that version (0.11.0, shipped with figgy-renderer 0.17.0, and
0.10.0, shipped with figgy-renderer 0.16.1), taken from crates.io with the
`serde` feature and serde_json 1.0.151. There is no trailing newline.

The documents are the default `Config`, a `Config` whose legend lists one
entry per series and which has a colourbar, a `Config` with edited axes,
grid, legend, title, selections, colourbar and draw style, every `DrawStyle`,
`Vec<SeriesConfig>` lists covering every `DataRenderType` variant of those
releases, and `RadialChart`, `CategoricalChart` and `BoxPlotChart`, each at
its default and edited. Only the 0.11.0 edited `Config` differs from its
0.10.0 twin: it sets `RichText.align`, which 0.10.0 does not have.

They were produced by a throwaway program outside the repository that depends
on `figgy-model = "=<version>"`, builds each document with that release's own
types and helpers, writes it, and checks that the release parses it back to
the same bytes. `tests/released_documents.rs` parses every file here with the
current types and requires the same bytes back. Do not edit or regenerate
these files: they record what was published.
