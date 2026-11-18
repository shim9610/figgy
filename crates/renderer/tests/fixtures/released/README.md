# Documents written by released versions

Each `model-<version>-<name>.json` file holds the exact bytes that
`serde_json::to_string` returned for one document built with the published
figgy-model of that version (0.12.1, the latest 0.12.x on crates.io; 0.11.0,
shipped with figgy-renderer 0.17.0; and 0.10.0, shipped with figgy-renderer
0.16.1), taken from crates.io with the `serde` feature and serde_json
1.0.151. There is no trailing newline.

The documents are the default `Config`, a `Config` whose legend lists one
entry per series and which has a colourbar, a `Config` with edited axes,
grid, legend, title, selections, colourbar and draw style, every `DrawStyle`,
`Vec<SeriesConfig>` lists covering every `DataRenderType` variant of those
releases, and `RadialChart`, `CategoricalChart` and `BoxPlotChart`, each at
its default and edited. Only the 0.11.0 edited `Config` differs from its
0.10.0 twin: it sets `RichText.align`, which 0.10.0 does not have.

The 0.12.1 set covers the four charts of the former bounded group only:
`RadialChart`, `CategoricalChart`, `BoxPlotChart` and `RadarChart`, each at its
default and edited (the last three are read by the deprecated readers since
model 0.13.0). Radar and the legacy rich `chart_title` shipped in 0.12.0 without a
fixture. Each edited 0.12.1 document is its 0.11.0 twin parsed by 0.12.1
(radar, new in 0.12.0, is built field by field) with every field added since
set: a `chart_title` that uses every `RichText` and `RichSegment` field,
`active_layer`, an `annotations` layer whose items cover every
`AnnotationKind`, and the new `DecorationId` variants (`Annotation`,
`SpokeLabel`). The edited box plot also gives its notched series' third box a
`median_ci`, which 0.12 validation requires. Every 0.12.1 document passes that
release's `validate()`.

They were produced by a throwaway program outside the repository that depends
on `figgy-model = "=<version>"`, builds each document with that release's own
types and helpers, writes it, and checks that the release parses it back to
the same bytes. `tests/released_documents.rs` parses every file here with the
current types, saves it, and requires the save to read back as the same value
and to re-serialize unchanged. `tests/released_documents_render.rs` exports
every chart document here at 1x and 2x before and after that save and requires
the same pixels. The current JSON may differ from these bytes where a later
release gave a fact one owner (the CHANGELOG's migration notes say where). Do
not edit or regenerate these files: they record what was published.

The edited radial documents set `decorations.data_area` to start at y = 60
under a title. Every release since 0.16.1 makes a titled radial chart's title
band at least 70 px tall and refuses an explicit plot area outside the
measured plot region, so no release draws these three documents as written
(`validate()`, which the generator checked, does not know the canvas). The
render test therefore checks that they are refused for the same reason before
and after a save, and compares pixels with the automatic plot area.
