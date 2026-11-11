# Release history

`figgy-renderer` and `figgy-model` are the Rust library packages. `figgy` in this
history is the browser product in `crates/web` (the wasm-bindgen wrapper and the
`<figgy-chart>` element), built from this repository with its own version; earlier
entries call it the browser integration example. Its version does not imply a
deployment of the separate [figgyplot.com Studio](https://figgyplot.com/) application.

Publication statements in older source-release entries describe those releases
at the time. The first crates.io publication is recorded below.

[Usage and gallery](README.md) · [Backend verification](ci/STATUS.md)

<a id="unreleased"></a>

## Unreleased — renderer 0.19.0 / model 0.13.0 / figgy 0.16.0

Collected on `integration/0.19`; the release entry replaces this section.
Every breaking change below carries its migration. Serialized documents
written by model 0.12 and earlier parse, read back as the same value after a
save and export the same image before and after it. Where a fact that was
stored twice now has one owner (#160), which the migration notes name, the
document is written in the new form and re-serializes unchanged from then on;
every other document still re-serializes byte for byte. The new keys (`"math"`, `"math_node"`, `"math_parent"`, `{"Plain":{}}`,
`categories`, `cross_at`, `bar_groups`, `log_ticks`, `line_offsets`,
`grid_shape`, the radar series style's `fill_opacity`, the value labels'
`offsets`, the `value_label` selection,
the `CategoryBars`, `BoxPlot`, `Radar`, `Streamlines`, `RangeBars` and
`Ternary` series kinds, the box plot's `violin` and `points`, `ternary`,
the ternary series' `region`, `view_3d`, the heatmaps' `surface`, the
axes' `breaks`, the `EnergyLevels` series kind, a value label selection's
`part`, a category's `rich_label`, the axes' `category_groups`, the
chart's `plot_background`, the xy series' `stem`, the `TriMesh` series
kind and the `mesh_triangle` selection, the axes' `category_window`, the
breaks' `sizing`, the energy-level `bands` and `band` style, a
subplot's `marker`, the energy-level `wavelength_labels`, a text
annotation's `align`, the tick labels' `clearance` and the axes' `unit`)
are additive.

<a id="unreleased-model"></a>

### Model (`figgy-model` 0.13.0)

- **Series constructors and modifiers** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature, `SeriesConfig` gains one constructor per chart family (`line`, `scatter`, `scatter_line`, `stem`, `histogram`, `heatmap`, `contours`, `hexbin`, `rgb_image`, `vectors`, `streamlines`, `category_bars`, `box_plot`, `range_bars`, `radar`, `ternary`, `parallel`, `mesh`, `energy_diagram`, `feynman_diagram`, and `of_kind`), each naming its columns and drawing with the host style defaults. Chainable modifiers set the common fields: `label`, `color`, `width`, `dash`, `marker`, `fill_to`, `x_errors`, `y_errors`, `horizontal` and `z`. `MatrixRef::columns_are_x`, `columns_are_y` and `with_layout` name a matrix. These are methods on the document types: the series stays the single source of truth.

- **More series modifiers and chart parts** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature, `SeriesConfig` also takes one-line modifiers for what most charts of a kind set: `with_line` (a line through the markers of an x-y or ternary series), `no_markers`, `fill_to`, `fill_between`, `fill_color`, `interpolation`, `with_stem`, `error_bars`, `error_color`, `color_by` (markers, parallel, streamline and point vector series), `size_by`; `bar_border` and `bar_gap` (histograms); `range_colors`, `whiskers`, `connector` and `bar_labels` (range bars; `bar_labels` also category bars); `violin` and `outliers` (box plots); `shading`, `opacity`, `filled` (a fill under contours), `contour_levels` and `contour_labels` (fields and meshes); `vector_scale`, `arrow_head` and `pivot` (vectors); `region` (ternary). Every other field is set in a closure on its sub-style: `style_line`, `style_markers` (a size map's scale, for one), `style_stem` (a stem's own colour), `style_bars`, `style_vectors` (the minimum length), `style_contours`, `style_surface`, and `style_range_bars` and `style_box_plot`, which hand over every field of their family as `RangeBarsParts` and `BoxPlotParts` (`&mut` references; progress, slot width, raw points, per-box overrides). `horizontal()` also turns a stem sideways; a horizontal histogram names its columns swapped. New constructors `vectors_grid` (matrix components) and `vectors_3d`, and `RadialChart::with_title`, a title the radial chart accepts. Chart parts: `Color::with_alpha`; `Annotation::reference_line`, `span`, `text` and `text_rich` with `with_label`, `with_line_style`, `with_box` and `with_align`; `AxisOptions::overlay_y`; `Subplot::new` with `with_link_x` and `with_link_y`; and on the renderer's `Chart`, `with_annotation`, `with_label_format`, `without_grid`, `with_colorbar_title`, `with_polar` and `with_subplot`. A modifier or closure that does not apply to a series' family leaves it as it is.

- **Math in labels, as matplotlib writes it** ([#272](https://github.com/shim9610/figgy/issues/272)). `text::rich_segments_from_label(&str)` and `RichText::from_label(text, color, size, font)` read a label the way matplotlib does: `$…$`, `\(…\)` and `$$…$$` spans are TeX (the segments `RichText::from_tex` gives), `\$` is a dollar sign, text outside math maps Unicode super- and subscripts as `rich_segments_from_text` does, and a string whose TeX does not parse is shown as written, with each `\$` as `$` (no panic, no error). Every convenience function that takes text as `&str` converts it through this one function: `SeriesConfig::label`, `Annotation::text`, `AxisOptions::overlay_y` and `RadialChart::with_title`. The SSOT types and their constructors stay explicit: `RichText::plain` is plain text and `RichText::from_tex` is math. Breaking for text that holds a `$`: see the renderer entry for the migration.

- **Tick labels make way at corners and break marks** ([#272](https://github.com/shim9610/figgy/issues/272)). Where two axes of a rectangular chart meet, one axis' tick labels yield to the other's (a y axis to the x axis, an x axis only to a categorical y axis; a categorical axis never yields), and on a 3D chart the z labels yield to the floor's labels: a label whose box (its width by its digits' height, 0.7 em) overlaps the other axis' label nearest the corner is dropped. A label whose box overlaps a slash of its axis' own break mark is dropped too. Two boxes overlap here when they come within a small visual gap of each other, closer than it both across and along the screen: 2 px in total between two labels in the default 18 px tick label font, each box's half read as that share of its own font, so `(a + b) / 18` px between an `a` px and a `b` px label, scaling with the display or export scale. Boxes 1 px apart at 18 px count and labels the gap apart or more stay, so labels that look separate are kept; the decision is made at rest, so a drag moves the kept set as one; a 2x export keeps the labels of 1x; and an axis never drops below two labels. Drawing, hit testing, selection and auto layout see the same labels. The new `LabelStyle::clearance: LabelClearanceMode` chooses it per axis: `Auto` (the default, omitted from JSON, so older documents parse and re-serialize byte for byte) as above, `Keep` drops no label for a corner or a break mark. At a corner the yielding axis' setting alone decides: a `Keep` y axis keeps its corner label, a `Keep` x axis does not stop the y label from yielding. **Migration:** a `LabelStyle { .. }` struct literal or exhaustive pattern adds `clearance`: `LabelClearanceMode::Auto` for the default, `Keep` to draw every label as model 0.12 did.

- **A click goes to the element the pointer is on** ([#272](https://github.com/shim9610/figgy/issues/272)). `HitMap::hit_test` no longer takes the topmost element whose padded hit region holds the point: where the padded regions of several elements overlap, an element whose own, unpadded geometry holds the point wins over one that reaches it only with its selection padding, among padding-only hits the one whose geometry is nearest wins, and ties keep the topmost registration. A point within one pixel of an element's geometry (`select::INK_FRINGE`, the reach of an antialiased edge or a border straddling its box) counts as on it. Backgrounds (`Selectable::is_background`: the data area, a subplot's frame, a decoration map's plot region) take no part: one answers only where no element above it holds the point, so a label's padding still grabs the label over the plot. Hit testing, selection and the start of a drag come through this one decision, made in one pass that keeps only the best candidate. New, both additive: `Selectable::is_background` (default `false`) and `SelectionGeometry::hit_distance`, how far the point stands from the element's own geometry where its hit region holds it.

- **Axis units: an axis prints its values in another unit** ([#273](https://github.com/shim9610/figgy/issues/273)). `AxisOptions::unit: Option<AxisUnit>`, with `AxisUnit { scale, offset }`, makes the axis print `shown = scale * value + offset` without the host converting the data: 16-bit ADC counts as volts (`scale = 5/32768`), kelvin as degrees Celsius (`offset = -273.15`), seconds as milliseconds. The data, `min`, `max`, `breaks`, `cross_at`, annotations and picks stay in data units. The ticks are planned in shown units over the shown range (`AxisOptions::shown_range`), so the labels are round numbers after the conversion (`0, 0.5, 1.0 V`, not converted counts), and each tick is placed at its data position `(shown - offset) / scale`: `tick::major_ticks` and `minor_ticks` walk the shown grid, still in increasing data order for a negative scale, and `tick::shown_tick_value` gives the walked value a label prints, so `0.1 + 0.2` noise never reaches a label. `major_spacing` is read in shown units; `auto_ticks`, `auto_significant_digits` and the range helpers plan for the shown range. The label format (`Decimal`, `Percent`, `Power`, `Plain` and the rest) applies to the shown value. A logarithmic axis walks decades of the shown value. Every consumer of the tick plan sees the same ticks: drawing, grid lines, auto layout, hit testing, selection and drag, the corner and break-mark clearance, and SVG / PDF / PNG export, on the Cartesian axes, overlay axes, the colourbar axis, the polar radius (and angle) axis, ternary and 3D box axes. A linked subplot axis takes its target's unit with its spacing. `Config::validate_chart` refuses a scale that is not finite and non-zero (`RuleKind::AxisUnitScale`), an offset that is not finite (`AxisUnitOffset`), a logarithmic axis' unit unless `offset == 0` and `scale > 0` (`AxisUnitLog`), and a unit on a probability axis (`AxisUnitProbability`), an axis with `Timestamp` labels (`AxisUnitTimestamp`) or a categorical axis (`AxisUnitCategorical`); `AxisOptions::validate_unit` and `effective_unit` share that decision. With the `helpers` feature, `AxisOptions::unit(scale, offset)` sets it and chooses the spacing for the shown range. `None` is the default and is omitted from JSON, so older documents parse and re-serialize byte for byte. **Migration:** an `AxisOptions { .. }` struct literal or exhaustive pattern adds `unit: None` (or `unit: _`).

- **Themes** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature, `model::theme` holds `Theme`: a look written into the document once, never stored as a name. `ChromeTheme` covers the chart and plot fills, the draw style, the axis frame (`AxisPreset`), axis, label, title and legend colours, fonts and sizes, the grid, the legend box, the colourbar outline and the selection highlight. `SeriesTheme` has one part per family: a `ColorCycle`, line widths and marker sizes, bars, boxes and violins, field colour maps and contour labels, vectors, radar, range bars, ternary regions, 3D surfaces, energy levels, Feynman diagrams and the radial chart. Built in: `light` (the documented default), `dark`, `paper`, `dashboard` and `sketch`, also through `Theme::named`. `Theme::apply(&mut config, &mut series, SubplotScope)` writes the chrome of the chart, its overlays and the chosen subplots, and styles the series drawn there; the legend symbols follow the new series look. `apply_chrome`, `style_series` and `apply_radial` apply one part. A theme keeps ranges, scales, label formats, categories, axis breaks, the chart area, offsets, all text, the annotation layer, the layer structure, series ids, columns and labels, and the selection's refs. Energy-level and Feynman charts keep their own frame.

- **A fill for the whole chart area** ([#265](https://github.com/shim9610/figgy/issues/265)). `Config::background: Option<Color>` paints the chart area under everything the chart draws: the plot fill, the grid, the data and the decorations. `None` (key absent in JSON) is the transparent chart area every document had, so older documents read and write as before. A subplot fills its own chart area from its own `background`; overlay layers and line-offset members draw none. In the Milkyway style the deep-space backdrop takes its place, as it takes the plot fill's. An exhaustive `Config { .. }` literal or pattern adds `background`.

- **Host defaults for the series styles and the chart frame** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature, `Default` now covers `Config`, `ChartTitleOptions`, `AxisTitleOptions`, `GridOptions`, `ColorBarOptions`, `Legend` and `ChartArea` (each the value `default` builds), and the series styles a document always spells out: `Stem`, `DataScatterStyleConfig`, `DataErrorBarStyleConfig`, `DataBarStyleConfig`, `VectorStyle`, `FieldFillConfig`, `ContourLabelConfig`, `RangeConnector` and `SurfaceWireframe`. Strokes take the black, 1 px ink of the line default; bars fill with the first colour of the balanced palette. `AxisOptions` and `LabelStyle` have none, since their default differs by axis. Product code cannot read them (#266).

- **One source per document default; `Default` for hosts only** ([#266](https://github.com/shim9610/figgy/issues/266)).
  - **One source, one folder.** Every serialized type with a default now spells it once, in the crate-private `defaults/` folder (one file per area: frame, series, annotations, chart families, science, text): the value a field takes when a document omits it. This covers the 46 hand-written `impl Default`, the 55 `#[derive(Default)]`, the `default` module's values (`Config`, the axis, title, label, legend, grid and colour bar options, `ChartArea`) and the ten field defaults that serde read from functions scattered over the model. `#[serde(default)]` reads them through `default = "crate::defaults::name"`. `Violin::DEFAULT_BOX_WIDTH` keeps its value, now read from there. Documents parse and serialize exactly as before.
  - **New `helpers` feature (on by default).** Host conveniences live in the `helpers/` folder behind it: `impl Default` on those types and the `default` module (now `figgy_model::helpers::default`, still reachable as `figgy_model::default`). The model's and the renderer's own code, their unit tests included, cannot fall back on a default: `ci/verify.py static` builds both libraries and their unit tests without it.
  - **Named functions in place of defaults.**
    - `AnnotationLayer::empty()` and `DataSelectionsConfig::empty()` mean "nothing".
    - `Config::effective_bar_groups()` resolves an omitted `bar_groups` once. `BarGroups` is now `Copy`.
    - `SurfaceStyle::resolved(Option<&SurfaceStyle>)` gives the surface look an unset style draws with.
    - `RadialStyle::label_raster_key` and `RadialInteraction::label_raster_key` neutralize what the radial label raster ignores.
  - **Migration:** a build with `default-features = false` adds `features = ["helpers"]` to keep `Default` and `figgy_model::default`.

- **Wavelength labels written by the renderer, and corner notes** ([#258](https://github.com/shim9610/figgy/issues/258)).
  A `Transition` holds one wavelength, `wavelength: Option<f64>` and
  `wavelength_uncertainty: Option<f64>`, in place of `observed_wavelength`,
  `observed_uncertainty`, `ritz_wavelength` and `ritz_uncertainty`: the
  host writes the NIST column it chose. `EnergyLevelStyle.wavelength_labels:
  Option<WavelengthLabels>` gives every transition with a wavelength and
  no host `label` a drawn label: the value in a `PlainNumberFormat` (at
  most `MAX_WAVELENGTH_DECIMALS` = 17 fixed decimals), then ` {unit}` when
  the labels' own `unit: Option<String>` is set (the host's text: `nm`,
  `Å`, `nm (air)`; `None` writes the number alone), in the font, size and
  colour of its `text`. The labels own their unit. A host `label` wins.
  `Transition::drawn_label` is the one owner of the choice, and the
  placement, the emit, the pick, the drag and the vector export read the
  label through it, so a formatted label is placed, drawn, picked and
  dragged as a host label is. The renderer formats the host's value and
  nothing else: it converts no unit and chooses between no values.
  `AnnotationKind::Text` gains `align: NoteAlign` (nine points: corners,
  edge middles, centre; `TopLeft`, the old placement, by default and
  omitted), the point of the block that sits on the anchor.
  `AnnotationLayer::add_corner_note(text, corner)` adds a text item at a
  corner or edge of the plot, its block inside it, and returns its index:
  a note that owns its text, such as the unit of the chart's values, and
  is hit, selected, dragged, hidden, serialized and exported as any text
  item. New rules: `WavelengthLabels`, `WavelengthUnit`. Energy levels are
  new in this cycle, so no released document carries the replaced fields.
  **Migration:** write `wavelength` (and `wavelength_uncertainty`) in place
  of the four line fields; an `AnnotationKind::Text { .. }` literal names
  `align` (`NoteAlign::TopLeft` keeps the old placement).

- **Energy-level labels keep off other columns** ([#248](https://github.com/shim9610/figgy/issues/248)).
  After stacking, a level or band label that lies on another column's level
  stroke or band fill, on a limit line, or on another label moves: its
  stacked group (or collapsed run) as one, or a single label taken out of
  its stack, to its column's other side, above its stroke (a lone label),
  or up or down by quarters of a label height (a stack by at most one
  height, a lone label by two and a half), wherever it overlaps least
  inside the plot, with a leader line when it moved. A label the host
  dragged keeps its place. In the bundled He, Ar, Cu, U and every-level Ar
  scenes no level label overlaps any more (26 overlaps before in Ar, Cu and
  U, 29 in every-level Ar). No API or serialized field changes.
- **Break gaps in pixels; two broken axes meet as a cross** ([#249](https://github.com/shim9610/figgy/issues/249)).
  `AxisBreaks.gap` (a fraction of the axis length) is replaced by
  `gap_px: f32`, the gap's width in logical pixels (default 8, at most
  `MAX_AXIS_BREAK_GAP_PX` = 200; `Config::scaled` scales it), so the x and
  y gaps of one chart are the same width whatever the plot's aspect.
  Both were added in this unreleased cycle (#200), so no released document
  carries `gap`. Placing a value now needs the axis' drawn length:
  `layout::BreakMap::of(axis, len)` and `layout::axis_fraction(value,
  axis, len)` take it (the data area's width for x, its height for y), and
  `chrome::line::axis_fraction(value, axis, side, plot)` and
  `chrome::line::axis_len(side, plot)` give it from the plot;
  `BreakMap::cuts(axis)` is the gap-free map for questions that need no
  length (is a value in a cut, the per-piece tick plan). When both axes
  are broken and frame their pieces, each frame stops at the other axis'
  gap, so the gaps meet as a cross instead of one frame running across the
  other's gap: `chrome::line::emit_axis`, `emit_axis_strokes` and
  `each_axis_stroke` take the perpendicular axis (`across:
  Option<&AxisOptions>`). Each frame now stands inside its own piece at
  the piece's pixel edge, so the clear space between two frames is the
  gap's own width; the axis line's pieces and their slashes end on the
  same pixel edge, a tick at a piece's end stands on that piece's frame
  row, and a grid line on a gap edge stays inside its piece
  (`chrome::grid::emit_grid` takes the display or export `scale`), so a 2x
  export stays the 1x one doubled.
  **Migration:** replace `gap: f` with `gap_px: f * axis_length_px` (or
  drop it for the 8 px default), and pass the axis length to `BreakMap::of`
  and `layout::axis_fraction`.
- **Typed validation reasons** ([#215](https://github.com/shim9610/figgy/issues/215)).
  A refused rule is a `RuleKind` value (new module `rule`, re-exported at the
  crate root): one `#[non_exhaustive]` enum for the model's and the
  renderer's rules, a unit variant per rule (`PolarImage`,
  `StemsWithoutMarkers`, `HexagonalShading`, …) and variants with fields for
  the rules that report ids, counts or lengths
  (`CategoryBarColumnLength { column, len, categories }`,
  `InvalidMath { error }`, …). Its `Display` is the sentence the error
  carried before, byte for byte, and `RuleKind::code` the variant's name.
  `ConfigError` carries `kind: RuleKind` in place of `reason: &'static str`
  (`ConfigError::reason()` returns the sentence), and every model
  `validate` method and validation helper that returned
  `Result<_, &'static str>` returns `Result<_, RuleKind>`: the axes,
  categories, breaks, series kinds, styles, annotations, layers, polar,
  ternary, parallel, 3D, energy-level and Feynman rules, the legacy
  document readers and `RadialChart::validate`. The `*_REASON` constants
  (`ContourLabelConfig::INVALID_SPACING_REASON`, …) are `RuleKind` values.
  Errors are not serialized, so documents are unchanged. Migration: match
  `err.kind` (or `Err(kind)`) against a `RuleKind` variant instead of
  comparing strings; code that wants the text calls `to_string()` on it.

- **Feynman diagrams** ([#234](https://github.com/shim9610/figgy/issues/234)).
  A DataLess series kind, `DataRenderType::Feynman(FeynmanSeries)`: the
  topology is the data (vertices with an incoming, outgoing or internal
  role, propagators naming two vertices and a `ParticleLine`: plain,
  fermion, antifermion, photon, charged boson, gluon, scalar, charged
  scalar, ghost, Majorana, graviton, double line), with optional vertex
  positions, markers, bends, labels and momentum arrows.
  `FeynmanSeries::positions` places every unplaced vertex by a deterministic
  layout (legs on the early and late edges of the frame, the rest at the
  mean of their neighbours, solved by conjugate gradients; bubbles on a
  circle), and `auto_layout` commits it into `at`, the auto-fit pattern; a
  layout whose solver stops converging is an error, and so is working
  memory that cannot be allocated; `positions`, `extent`, `auto_layout`,
  `emit`, `pick`, `drag_vertex` and `drag_bend` return it. `FeynmanSeries::emit`
  and `pick` place, draw (through `Sink::path` for arcs, waves and coils)
  and pick a diagram from one placement, in at most four transient buffers
  (two for a pick of a committed diagram; text measuring allocates in the
  measure the caller passes). Every line is sampled and measured within the
  plot only, so a diagram zoomed in any amount draws and picks with the
  work of an ordinary view. A pick outside the plot answers nothing; a
  vertex answers within the pick distance of its marker and there wins over
  its own lines, so a leg end picks. Parallel lines fan within 170
  degrees, each on its own arc, however many. The picked part is drawn in
  `style.picked` (`color`, `width_extra_px`), a picked label boxed.
  `drag_vertex` (which commits the layout on the first drag), `drag_bend`
  and `drag_label` (by list and row, D7) edit it;
  `Config::validate_feynman` holds a series to a rectangular chart with a
  numeric `bottom_x`. Vertex and propagator ids are held to be unique. `PickedDataRef` gains `FeynmanVertex`,
  `FeynmanPropagator` and `FeynmanLabel` (serde kinds `feynman_vertex`,
  `feynman_propagator`, `feynman_label`), stepped by `step_selection` over
  the rows and the Feynman series. `SeriesConfig::is_dataless` names the
  energy-level and Feynman kinds. No count limit. Migration: an exhaustive
  match on `DataRenderType` or `PickedDataRef` adds the new arms.
- **A hidden axis preset** ([#234](https://github.com/shim9610/figgy/issues/234)).
  `AxisPreset::Hidden` switches off every axis line, tick and tick label
  and the grid, keeping titles, ranges and margins: the frame of a figure
  that is its own picture, such as a Feynman diagram. The web wrapper's
  `AxisPreset` mirror gains it. Migration: an exhaustive match on
  `AxisPreset` adds the arm.
- **Stroked paths in the sink** ([#234](https://github.com/shim9610/figgy/issues/234)).
  `Sink::path(slot, &[PathSegment], &StrokeStyle)` places a stroked path of
  straight and cubic segments, stroked as one path so a dash runs along it
  without restarting: the coming Feynman propagators' arcs, waves and coils.
  `PathSegment` (`Move`, `Line`, `Cubic`, `Close`) moves from the renderer's
  sketch stroker into `model::sink`, and `DrawnPart::Path { points, width }`
  records every point a path names, control points included, so its
  extent bounds the curve. The canvas sink strokes it with tiny-skia, the
  vector export writes it as one SVG `<path>` or PDF path with its cubics
  kept, and the geometry sinks bound it. Migration: an implementation of
  `Sink` adds `path` (a sink that ignores strokes adds an empty one, or
  forwards each segment to `stroke`); an exhaustive match on `DrawnPart`
  adds the `Path` arm.
- **Too many explicit contour label anchors is an error** ([#107](https://github.com/shim9610/figgy/issues/107)).
  `ContourLabelConfig::validate` rejects an `anchors` list longer than the new
  `MAX_CONTOUR_LABEL_ANCHORS` (1024) with
  `ContourLabelConfig::TOO_MANY_ANCHORS_REASON`. Registration, series edits,
  prepare and export report it as `InvalidSeriesConfig`. Before, the renderer
  kept the first 1024 valid anchors and dropped the rest silently. The limit
  is unchanged; the renderer's label buffer (`MAX_CONTOUR_LABELS_TOTAL`) now
  takes its value from this constant. Migration: a host that passed more than
  1024 anchors keeps at most 1024 itself.

- **A legend symbol names its series** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `RichSegment` gains `series_id: Option<String>`, and
  `series_symbol_segments` sets it on the symbol it makes.
  `update_legend_symbols_preserving_text` pairs each entry with the series
  its symbol names. Before, it paired the i-th entry with the i-th series.
  So an entry now keeps its own mark when series are reordered or when a
  series has no entry; before, a series without a label shifted every later
  entry onto its neighbour's mark. New `legend_entry_of_series(content,
  series, id)` and `legend_entry_series_id(content, index)` find an entry
  by its series. The key is additive (written only when set); a symbol
  without it, from an older document, still pairs by position and names
  its series once it is restyled. Migration: code that builds a
  `RichSegment` with every field listed adds `series_id: None`.
- **`Column` holds values only** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Column<T>` loses its `min` and `max` fields: the renderer measures a
  column's statistics while it uploads the values, so a stored copy could
  only disagree with them. The upload helpers `UploadValues::min`/`max` and
  `BoxColumn::min`/`max`, which existed to supply them, are removed.
  Migration: write `Column { data }`; drop the code that computed the two
  bounds.
- **The model places the axis chrome** ([#160](https://github.com/shim9610/figgy/issues/160)).
  Tick planning, formatting and label pruning, the timestamp ticks, and the
  placement of the grid, the axes with their ticks and tick labels and the
  colourbar's axis and labels (rectangular, polar, ternary, 3D box and
  parallel charts) moved from the renderer into the model's
  `chrome` module, measured through `MeasureText` alone. Under any measure
  the drawn tick labels and axis strokes now have bounds, hit tests and drag
  containment; before, only the renderer's measure placed them and any
  other left tick labels without bounds. The renderer paints the same
  placement, so its raster, hit tests and exports are unchanged.
  `MeasureText::emit` is removed; `MeasureText::measure_plain(text, family,
  size)` is new, with a default that measures the text as a plain
  `RichText`. Migration: a `MeasureText` implementation drops its `emit`;
  one whose plain-text measurement differs from its rich-text one
  overrides `measure_plain`.
- **The deprecated readers' companion types are deprecated too** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `BarSeries`, `BoxSummary`, `BoxPlotDirection`, `BoxPlotSeries`,
  `RadarTarget`, `RadarGridShape`, `RadarStyle` and `RadarSeries` exist only
  inside `CategoricalChart`, `BoxPlotChart` and `RadarChart`, deprecated
  since 0.13.0, and leave with them. Code that reads a legacy document
  through them adds `#[allow(deprecated)]`, as for the readers.
- **Unused items removed** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `DataConfig` (a lone `data_id`), `DataCell` (a bundle of columns nothing
  built), `default::default_rich_text` (an empty black 12 px `RichText`) and
  `BoundedTitleOptions::has_text` had no user. A host that used one writes
  it in place: `RichText { segments: Vec::new(), color: Color::BLACK,
  font_size: 12.0, font: String::new(), align: TextAlign::Left }`, and
  `title.visible && !title.text.segments.is_empty()`.
- **One radial title** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `RadialChart::title`, the plain string drawn in the chart's font when
  `chart_title` was absent, is removed with `normalize_title`:
  `chart_title` is the chart's only title. `RadialChart::set_title_text`
  makes a plain title in the chart's current font at 1.5 × `font_size` and
  in `label_color`, keeping an existing title's visibility; an empty text
  removes the title. A document that still carries `title` is read into
  `chart_title` in the document's font when it has none (an empty `title`
  reads as no title, and a `title` beside a `chart_title` is dropped, as it
  was never drawn), and only `chart_title` is written. Behaviour change: a
  plain title no longer follows a later edit of `font_family`, `font_size`
  or `label_color`; call `set_title_text` again. Migration:
  - `chart.title = text.into()` → `chart.set_title_text(text)`;
  - `RadialChart { title: text.into(), .. }` → build the chart, then
    `set_title_text(text)` once its font is set;
  - `chart.normalize_title()` → nothing (the title is already rich).
- **One selection per chart** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Config::picked_points` (`PickedPointsConfig`, `PickedPointRef`), the
  point-only selection drawn beside `picked_data`, is removed:
  `Config::picked_data` holds every selection, points as
  `PickedDataRef::Point`. A document that still carries `picked_points` is
  read into `picked_data`: its refs become `point` refs after the ones
  `picked_data` lists, and when the document has no `picked_data` its
  `visible`, `ring_color`, `ring_width_px` and `radius_extra_px` become
  `visible`, `highlight_color`, `outline_width_px` and
  `point_radius_extra_px`. Beside a `picked_data`, the points take its look,
  and hidden points (`visible: false`) are dropped. Only `picked_data` is
  written, so such a document re-serializes in the new form. Migration:
  - `PickedPointsConfig { visible, refs, ring_color, ring_width_px,
    radius_extra_px }` → `DataSelectionsConfig { visible, refs,
    highlight_color, outline_width_px, point_radius_extra_px,
    ..Default::default() }`;
  - `PickedPointRef { source_id, series_id, point_index }` →
    `PickedDataRef::Point { source_id, series_id, point_index }`;
  - a `Config { .. }` literal or pattern drops `picked_points`.
- **One coordinate system per chart** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Config`'s four optional systems (`polar`, `ternary`, `view_3d`,
  `parallel`), whose exclusivity three validators checked, are one field,
  `Config::coordinates: Coordinates` (`Cartesian`, `Polar`, `Ternary`,
  `View3d`, `Parallel`). `Config::polar()` / `polar_mut()` /
  `set_polar(Option<..>)` and their `ternary`, `view_3d` and `parallel`
  siblings read and set one system; `set_*(None)` makes a chart of that
  system rectangular and leaves another system alone. On the wire nothing
  changes: the system is written as the one key it always was, at its
  place among the chart's keys; a document naming two systems, which no
  renderer drew, is now refused when it is read. Migration:
  - `config.polar` → `config.polar()` (an `Option<&PolarOptions>`);
    `config.polar.as_mut()` → `config.polar_mut()`;
  - `config.polar = Some(p)` → `config.set_polar(Some(p))` or
    `config.coordinates = Coordinates::Polar(p)`;
  - a `Config { .. }` literal names `coordinates` instead of the four
    fields; `Coordinates::from_options(polar, ternary, view_3d, parallel)`
    builds it from the four options.
- **Composed render types** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The nine marker / line / error-bar combinations of `DataRenderType`
  (`Scatter`, `Line`, `ScatterLine`, `ScatterErrorbarX`, `ScatterErrorbarY`,
  `ScatterErrorbarXY`, `LineScatterErrorbarX`, `LineScatterErrorbarY`,
  `LineScatterErrorbarXY`) are one variant, `DataRenderType::Xy(XySeries {
  scatter, line, errors, stem })` (`stem`: #206, `None` for these), with
  `ErrorBars { x, y, style }`; the three
  field combinations (`Heatmap`, `Contour`, `HeatmapContour`) are
  `DataRenderType::Field(FieldSeries { matrix, fill, contour, surface })`.
  `XySeries::validate` and `FieldSeries::validate` hold the combinations a
  series can draw (markers or a line; error bars with markers; a fill or
  contours; a surface look with a fill), and `Config::validate_series`
  checks them. On the wire nothing changes: documents keep the
  one-variant-per-combination form and parse and re-serialize byte for
  byte; serializing a combination with no written form fails. Migration:
  - `DataRenderType::Scatter { scatter }` →
    `DataRenderType::Xy(XySeries { scatter: Some(scatter), line: None, errors: None, stem: None })`;
    `Line { line }` → `line: Some(line)`; `ScatterLine` → both;
  - `ScatterErrorbarX { scatter, err_x, err_style }` →
    `Xy(XySeries { scatter: Some(scatter), line: None, errors:
    Some(ErrorBars { x: Some(err_x), y: None, style: err_style }), stem: None })`, and
    likewise the `Y`, `XY` and `LineScatterErrorbar*` forms;
  - `Heatmap { matrix, fill, surface }` →
    `Field(FieldSeries { matrix, fill: Some(fill), contour: None, surface })`;
    `Contour { matrix, contour }` → `fill: None, contour: Some(contour),
    surface: None`; `HeatmapContour` → both;
  - a `match` reads the parts instead of listing combinations, for example
    `DataRenderType::Xy(xy) => xy.line.as_ref()`.
- **Stem and lollipop plots** ([#206](https://github.com/shim9610/figgy/issues/206)).
  `XySeries` gains `stem: Option<Stem>`: stems from a baseline to each
  point, drawn with the series' markers. `Stem { baseline, direction,
  color, width, line_style }`: the baseline in data units of the value
  axis, `direction` (`CategoryBarDirection`, `Vertical` along y by
  default, `Horizontal` along x for a lollipop), the stroke colour, width
  in logical pixels and dash (`Solid` by default). The column roles never
  swap. On the wire the eight marker variants (`Scatter`, `ScatterLine`,
  `ScatterErrorbar*`, `LineScatterErrorbar*`) carry an optional `stem`
  key, omitted when `None`, with `direction` and `line_style` omitted at
  their defaults. `XySeries::validate` refuses stems without markers, a
  non-finite baseline and a width that is negative or not finite; polar
  charts refuse stems ("stems are not drawn on polar axes"), and so do 3D
  charts ("stems draw flat: not on a 3D chart"). `ColorCycle` recolours a
  stem with the rest of the series. Migration: an `XySeries { .. }`
  literal adds `stem: None`, and a pattern that names every field adds
  `stem` or `..`.
- **Heatmaps and contours on polar charts** ([#208](https://github.com/shim9610/figgy/issues/208)).
  `Config::validate_series_on_polar` accepts a field series. It refuses a
  hexagonal lattice ("a hexagonal grid is drawn on a rectangular chart")
  and a field on a `GridShape::Polygon` grid ("a polar heatmap's cells
  follow circular rings: not on a polygon grid"). An image keeps its
  refusal under its own reason, "images are not drawn on polar axes"
  (previously "field and contour series are not drawn on polar axes").
  No type or serialized key is added: a polar heatmap document parses and
  re-serializes byte-identically.
- **Probability axis scales** ([#209](https://github.com/shim9610/figgy/issues/209)).
  `AxisScale::Probability(ProbabilityScale)` places a probability `p` in
  `(0, 1)` at `F⁻¹(p)`: `Normal` (probit, Wichura's AS241 `PPND16`, its
  inverse through Cody's `erfc`), `Weibull` and `Logistic`, in the new
  module `probability`. `AxisScale::forward` / `inverse` are the one CPU
  scale map, which the geometry (`axis_fraction`, `axis_cross`, the break
  map), the annotation space (`AxisMap::Probability`), `shift_axis` (line
  offsets in units of `F⁻¹(p)`) and the ticks read.
  `guarded_probability_range` and `PROBABILITY_RANGE_FALLBACK` (0.001 to
  0.999) guard an unvalidated range. Ticks come from the range: the tail
  `min(p, 1 − p)` walks the log ladders (decades, 1-2-5, 1 to 9) mirrored at
  0.5, with 0.5 on every ladder, the sparsest with four majors, then a linear
  grid in `p` for a narrow window (`ProbabilityTicks`, a `Copy` walk bounded
  by `MAX_LOG_DECADES`); minors are the next denser ladder; label pruning
  demotes along the tails (`ladder_multiple`). `compute_nice_ticks` widens a
  probability range to the next tail decade and returns the new
  `TickError::OutsideProbability` for a range outside `(0, 1)`. The new
  `LabelFormat::Percent` writes `25%`, and on a probability axis `0.1%`,
  `50%`, `99.9%` with the decimals each tail needs. Validation: bounds and
  cut ranges strictly inside `(0, 1)`; a categorical axis, a colourbar axis,
  a parallel axis, the polar radius and the value axis of category bars and
  box plots (the position axis of range bars, which tested
  `== Logarithmic`) refuse the scale; polar angle, ternary, 3D and
  hexagonal grids already accepted only linear axes. JSON:
  `"scale": {"Probability": "Normal"}`; the unit variants stay strings, so
  older documents parse and re-serialize byte-identically. Migration: a
  `match` on `AxisScale`, `AxisMap`, `LabelFormat` or `TickError` adds the
  new variant; `layout::span_fraction(value, min, max, logarithmic: bool)`
  takes `&AxisScale` (`false` → `&AxisScale::Linear`, `true` →
  `&AxisScale::Logarithmic`).
- **3D vector fields** ([#211](https://github.com/shim9610/figgy/issues/211)).
  `VectorComponents::Points` gains `w: Option<ColumnId>`, the z component.
  On a 3D chart a point vector draws when it names its anchor height in
  `SeriesConfig.z_column` and its z component in `w`;
  `Config::validate_view3d_series` refuses each missing one with its own
  reason ("a vector series on a 3D chart names its anchor height column
  (z_column)", "... names its z component (w)") and a grid vector ("a grid
  vector draws flat: not on a 3D chart"). On a flat chart both are kept and
  unused. `Config::drawn_z_column` names a 3D point vector's height, the
  step count includes the height and `w` on a 3D chart, and
  `SeriesConfig::visit_columns` visits `w`.
  `VectorStyle::fit_scale_3d(u_bounds, v_bounds, w_bounds, target)` sizes the
  longest vector the bounds allow, `hypot(max |u|, max |v|, max |w|)`. On the
  wire `w` is omitted when `None`, so older documents parse and re-serialize
  byte-identically. Migration: a `VectorComponents::Points { u, v }` literal
  becomes `Points { u, v, w: None }`, and a pattern that names every field
  adds `w` or `..`.
- **A box axis title clears its widest tick label** ([#211](https://github.com/shim9610/figgy/issues/211)).
  The title of a 3D box axis sat a fixed band (about 1.4 label heights)
  past its edge. On a near-vertical edge a wide label (`3.00` on the z
  axis) reaches farther along the normal than that, and the title ran into
  it. The band is now at least the widest drawn label's extent along the
  edge's normal. `Config::box_title_placement(k, offset, extents,
  label_reach)` takes that reach (`None` keeps the fixed band); the
  decoration layout, which the draw and the hit map share, passes the
  measured one. Migration: a direct caller adds
  `None`, or the reach of its own labels.
- **The columns a series reads are public** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `SeriesConfig::visit_columns` visits every plain column a series reads
  (coordinates, marker, error-bar, grid, component, colour and further axis
  columns) and `SeriesConfig::visit_composite_columns` a box plot's record
  and ragged columns. The renderer's column bookkeeping and the browser
  product read the same list, which before were two copies that disagreed
  on a box plot's record. Additive.
- **One home for the chart and series rules** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Config::validate_chart` holds every rule on the chart itself (a non-empty
  target, finite and increasing axis ranges positive on a log scale, the
  colourbar, overlay and subplot rules, the layout, the math in titles and
  legends, the annotations, the line offset layout), which the renderer
  kept before. `Config::validate_series` holds every rule on a series list
  that needs no column data: the series ids, the category bar, box plot,
  range bar, radar, ternary, parallel, 3D, hexagonal, image and line offset
  rules, and `Config::validate_series_on_polar` per series. Both return a
  `ConfigError { series_id, field, reason }`; a chart-wide rule names the
  first series whose addition breaks it instead of the renderer guessing.
  Additive; nothing on the wire changes.
- **No panic paths** ([#152](https://github.com/shim9610/figgy/issues/152)).
  The TeX parser bounds group nesting at 64 levels and answers deeper input
  with `TexErrorKind::TooDeep` instead of overflowing the stack.
  `ParallelOptions::validate_axis_count` refuses a parallel chart with fewer
  than two axes; `ParallelOptions::validate` and the renderer's config
  check call it, so such a chart is refused when it is set instead of
  failing at draw time. Migration: code that set a
  one-axis parallel chart now gets the reason. Nothing on the wire changes.
- **Shape annotations**
  ([#138](https://github.com/shim9610/figgy/issues/138), #58).
  `AnnotationKind::Rect { from, to, style }`, `Ellipse { from, to, style }`
  and `Polygon { vertices, style }` with `ShapeStyle { fill, border, below }`:
  the shape's screen polygon clipped to the plot area
  (`AnnotationGeometry::Polygon`, `annotation::clip_polygon`), a rectangle's
  eight box handles (`AnnotationHandle::Box`), a polygon's vertex handles
  (`AnnotationHandle::Vertex`, `AnnotationElement::hit_vertex`), the hit
  test anywhere inside the polygon (`PartOutline::Polygon`, non-zero
  winding, `select::polygon_contains`). `Sink` gains `polygon(slot, points,
  fill, border)` and `DrawnPart` / `PartOutline` a `Polygon(Vec<[f32; 2]>)`
  variant; both lose `Copy` (they are `Clone`). Migration: every `Sink`
  implementation adds `polygon`; code that copied a `DrawnPart` or a
  `PartOutline` clones it; an exhaustive match on `AnnotationKind`,
  `AnnotationGeometry`, `AnnotationHandle`, `DrawnPart` or `PartOutline`
  adds the new arms. Additive on the wire.
- **Roses: category bars on a polar chart**
  ([#133](https://github.com/shim9610/figgy/issues/133), #58).
  `Config::validate_category_bars` accepts `CategoryBars` series on a polar
  chart with a categorical angle axis (`bottom_x`) and a linear radius axis:
  the bars are wedges, a rose. The group direction stays `Vertical`; a
  `Horizontal` group on a polar chart is refused ("a rose fans its bars
  around the centre: the group direction on a polar chart is vertical").
  Migration: code that relied on "category bars are not drawn on polar
  axes" now gets a drawn rose or the horizontal reason. Nothing on the
  wire changes.
- **Parallel coordinates**
  ([#141](https://github.com/shim9610/figgy/issues/141), #58).
  `Config.parallel: Option<ParallelOptions>` (`ParallelOptions { axes }`,
  `ParallelAxis { min, max, inverted, scale, label_offset }`, module
  `parallel`): one vertical axis per category of `bottom_x`, each with its
  own range; `ParallelAxis::fraction` places a value on its axis and
  `axis_options` gives the tick planner its options.
  `DataRenderType::Parallel { columns, line, color_column }` draws one
  polyline per row across the axes; `DataRenderType::parallel()` answers
  `ParallelRef`. `Config::validate_parallel` (two or more axes with finite
  ordered ranges, a categorical `bottom_x` with one category per axis,
  `left_y` linear over `0..1`, not polar, ternary or 3D, no overlays or
  line offsets, the `Precise` style, no subplot axis links) and
  `Config::validate_parallel_series` (only parallel series on a parallel
  chart and vice versa, one column per axis with `x_column` / `y_column`
  the first two, a solid linear unfilled line, a colour column with
  `Config.colorbar`); `validate_view3d_series` rejects it too.
  `Element::ParallelAxisLabel(usize)` and `ParallelAxisLabelElement`
  (`parallel_axis_labels_{i}`) are each axis's tick labels, registered by
  `HitMap::for_active_layer` on a parallel chart and dragged into the
  axis's `label_offset` (`nudge`). `step_selection` walks the rows bounded
  by every axis column; the legend symbol is the line's. Additive on the
  wire (`parallel` omitted when `None`, `"Parallel"`); a `Config` literal
  adds `parallel`, an exhaustive `DataRenderType` or `Element` match adds
  the arm.
- **Sunbursts** ([#140](https://github.com/shim9610/figgy/issues/140), #58).
  `RadialSlice.children: Vec<RadialSlice>` (omitted when empty): a chart
  whose slices have children is a sunburst, its tree's rings drawn outward
  from the centre, each node over its share of its parent's span.
  `RadialTarget { group, index }` is then the ring and the node's position
  on it in document order; `slice()`, `slice_mut()`, `step_selection`
  (ring by ring outward), `format_labels` (a node's percentage is its share
  of its parent) and `labels_have_text` walk the tree, and `group_count()`,
  `group_len()`, `node_count()` and `node_span()` are new. The rules
  (`validate`): a flat chart, no `split`, children summing to at most the
  parent, at most `MAX_SUNBURST_DEPTH` (8) rings of at most
  `MAX_RADIAL_SLICES` (64) nodes, the limits approved on #140. Additive on
  the wire; a `RadialSlice` literal adds `children`.
- **Colour images (imshow)**
  ([#139](https://github.com/shim9610/figgy/issues/139), #58).
  `DataRenderType::RgbImage { red, green, blue, range, shading, opacity }`:
  three `MatrixRef`s on one lattice drawn as a colour image, each channel
  mapped linearly from `range` (`[0, 1]` by default) to 0..1 and clamped,
  no colourbar. `DataRenderType::rgb_image()` answers the three grids;
  `Config::validate_rgb_image_series` holds the channels to one lattice
  (the same column count, orientation and grid layout), a finite ordered
  `range`, an `opacity` in `0..=1` and a flat chart, and
  `validate_view3d_series` rejects it too. `range`, `shading` and `opacity`
  are defaulted on the wire. Additive (`"RgbImage"`); an exhaustive
  `DataRenderType` match adds the arm.
- **Hexagonal grids (hexbins)**
  ([#134](https://github.com/shim9610/figgy/issues/134), #58).
  `GridLayout::Hexagonal`: a `Heatmap`'s grid as `n` columns by `m` rows
  of flat-topped hexagons centred at `(x[c], y[r] + (c mod 2) · py / 2)`,
  the odd columns raised by half the row pitch; `MatrixRef::coordinate_cells`
  counts one cell per coordinate. `Config::validate_hexagonal_series`
  holds it to a flat-shaded `Heatmap` on linear axes of a flat chart (a
  contour, an interpolated fill, a grid vector and a 3D view read a
  rectangular lattice), and `validate_view3d_series` rejects it too.
  Additive on the wire (`"Hexagonal"`); an exhaustive `GridLayout` match
  adds the arm.
- **No count caps on column-backed series**
  ([#107](https://github.com/shim9610/figgy/issues/107) R1, R2, R3, R4,
  R10).
  A series backed by `ColumnSource` has no count limit; the only failure
  left is allocation when its columns are registered.
  `AxisOptions::validate_categories` takes any nonzero number of
  categories (ids, text and the linear scale are still checked), and a
  categorical axis walks one tick per category however many there are.
  `Config::validate_category_bars` and `validate_box_plots` no longer cap
  the series, bars or boxes of a group, `Config::validate_radar` no longer
  caps the radar series of a layer, and `LineOffsets::validate` no longer
  caps its members. The legacy readers follow, so a document they accept is one
  the series draws: `CategoricalChart::validate` and
  `BoxPlotChart::validate` take any number of categories and entries,
  `BoxPlotChart::validate` any number of outliers (each still finite and
  outside its whiskers), and `RadarChart::validate` any number of spokes
  from three up; all three take any number of series. Migration: the public
  constants `categorical::MAX_CATEGORIES`, `categorical::MAX_CATEGORY_BARS`,
  `categorical::MAX_BAR_SERIES`, `boxplot::MAX_BOX_SERIES`,
  `radar::MAX_RADAR_SERIES`,
  `boxplot::MAX_BOX_CATEGORIES`, `boxplot::MAX_BOXES`,
  `boxplot::MAX_BOX_OUTLIERS`, `boxplot::MAX_BOXPLOT_OUTLIERS`,
  `radar::MAX_RADAR_CATEGORIES` and `MAX_OFFSET_LINES` (also
  `data_config::MAX_OFFSET_LINES`) are removed; code that named them picks
  its own bound if it wants one. Code that matched the removed error
  strings ("a categorical axis needs 1 to 64 categories", "a bar group
  draws at most 512 bars", "a box group draws at most 512 boxes", "a bar
  group holds at most 16 category bar series", "a box group holds at most
  16 box plot series", "a radar chart draws at most 16 radar series", "a
  line offset group holds at most 32 series", the legacy readers' "limits"
  strings, which now read "at least 1 category and 1 series" and "at least
  3 categories and 1 series") no longer sees them; an empty categorical axis now fails with
  "a categorical axis needs at least one category".

- **Violins and rainclouds** ([#53](https://github.com/shim9610/figgy/issues/53),
  #58 item 23). `DataRenderType::BoxPlot` gains `violin: Option<Violin>`
  and `points: Option<BoxPoints>`. A `Violin` is
  `ViolinBody::Symmetric(ViolinHalf)` (one curve mirrored) or
  `ViolinBody::Split { low, high }` (a curve per side, at least one), with
  `box_width`, the box's share of the slot (default 0.25, 0 hides it). A
  `ViolinHalf` names the host's density curve as a ragged column (one group
  per category, ascending) and an index-aligned density column, its
  `density_scale` (`fit_density_scale` sets it from the density maximum)
  and a `ViolinStyle`. `BoxPoints` names two plain columns, each point's
  category position and value, with a `BoxPointsStyle` (colour, size,
  shape, filled, `jitter` and `offset` shares of the half slot). A violin
  without boxes leaves `y_column` empty. `validate_box_plot_series` holds
  the shares, sizes and widths in range, the scales finite and positive and
  the column ids distinct. `step_selection` walks a box plot's raw points
  by row, bounded by the shorter point column. `BoxPlotRef` gains
  `ragged_columns`, `plain_columns` and `draws_boxes`. The renderer
  computes no density; there is no limit on curve points, categories or
  raw points. Both fields are omitted from a document when unset. Code that
  builds `DataRenderType::BoxPlot` by struct literal adds `violin: None,
  points: None` (`BoxPlot` is unreleased).
- **A picked split violin names its half**
  ([#53](https://github.com/shim9610/figgy/issues/53) D1).
  `PickedDataRef::HistogramBin` and the renderer's
  `PickedData::HistogramBin` gain `half: Option<ViolinSide>`: `Some(Low)`
  or `Some(High)` for a split violin's half under the cursor, `None` for
  every other bin, a box and a symmetric violin. Selecting a half outlines
  that half alone; selecting the category (`None`) outlines the whole
  violin and its box. `step_selection` moves to a whole category
  (`None`). `ViolinSide` gains serde. The key is omitted when `None`, so
  documents parse and re-serialize byte for byte. Migration: a
  `HistogramBin { .. }` literal adds `half: None`, and a pattern that names
  every field adds `half` or `..`.
- **3D surfaces** ([#49](https://github.com/shim9610/figgy/issues/49)).
  `Config.view_3d: Option<View3d>` turns a chart into a box seen through a
  camera (`azimuth_degrees`, `elevation_degrees`, `distance`, `projection`
  and `box_aspect`), with x on `bottom_x`, y on `left_y` and z on
  `right_y`. A `Heatmap` on it draws as a surface, a `Contour` on the box
  floor and a `HeatmapContour` both: the data type is the heatmap's, so a
  chart switches between 2D and 3D by its view alone. `Heatmap` and
  `HeatmapContour` gain `surface: Option<SurfaceStyle>` (a colour grid, a
  wireframe, a material and its parameters, the highlight's brightness).
  `Config::view_matrix` is the one camera matrix; `Config::project_3d`,
  `Config::box_axis_edge`, `Config::box_title_placement` and
  `Config::box_back_panes` place the box chrome; `Config::orbit_view` and
  `Config::zoom_view` turn and zoom the camera, and a data-area drag
  orbits it. `Config::validate_view_3d` and
  `Config::validate_view3d_series` hold the rules (linear or logarithmic
  axes, not polar or ternary, no overlays or line offsets, Precise only;
  heatmaps, contours and heatmap contours only, continuous fills, no floor
  labels). Box axes drag along their edge's outward normal, tick labels in
  the edge frame and titles in screen space. `view_3d`, `surface` and
  `box_aspect` are omitted when `None`. Migration: a `Config { .. }`
  literal adds `view_3d: None`; a `Heatmap { .. }` or `HeatmapContour { .. }`
  literal adds `surface: None`, and a pattern that names every field adds
  `surface` or `..`.
- **3D scatter and line series** ([#132](https://github.com/shim9610/figgy/issues/132)).
  `SeriesConfig` gains `z_column: Option<ColumnId>`, the height column a
  `Scatter`, `Line` or `ScatterLine` series draws with on a 3D chart
  (`right_y`); `Config::validate_view3d_series` accepts those kinds with it
  (linear, no area fill) and rejects them without it, and
  `Chart::auto_fit_view_3d_z` unions the z columns. `z_column` is omitted
  when `None` and kept and unused on a flat chart. Migration: a
  `SeriesConfig { .. }` literal adds `z_column: None`, and a pattern that
  names every field adds `z_column` or `..`.
- **A 3D series' height column is one of its columns**
  ([#207](https://github.com/shim9610/figgy/issues/207)).
  `SeriesConfig::visit_columns` now visits `z_column` when it is set, so the
  enumeration every column check reads names it. New
  `Config::drawn_z_column(&series)` answers the height column that bounds a
  series' draw: its `z_column` when it is a scatter or line series on the 3D
  panel that owns it (the chart or its subplot), `None` on a flat one.
  `step_selection` stops at the shortest of x, y and that column.
- **Ternary region fills** ([#49](https://github.com/shim9610/figgy/issues/49),
  [#55](https://github.com/shim9610/figgy/issues/55)).
  `DataRenderType::Ternary` gains `region: Option<TernaryRegionStyle>`
  (`color`, `opacity`): the series' samples are the vertices of closed
  polygons, a NaN row separating them, filled under every series' markers
  and lines in series order. Within one series polygons combine by
  even-odd (a polygon inside another is a hole, an overlap is unfilled);
  regions that should overlap go in separate series. A ternary series now
  draws markers, a line, a region fill or any of them;
  `Config::validate_ternary_series` checks the fill's colour and opacity.
  The legend swatch carries the fill as a band. `region` is omitted when
  `None`. Migration: a `Ternary { .. }` literal adds `region: None`, and a
  pattern that names every field adds `region` or `..`.
- **Ternary plots** ([#55](https://github.com/shim9610/figgy/issues/55),
  #58 item 24). `Config.ternary: Option<TernaryOptions>` turns a chart into
  a triangle, and `DataRenderType::Ternary { c_column, scatter, line }`
  draws samples of three components named by `x_column`, `y_column` and
  `c_column`, unnormalized. The convention is fixed: x on `bottom_x` with
  its 100 % at the bottom-left corner, y on `left_y` at the top, c on
  `right_y` at the bottom-right, each increasing clockwise; `top_x` is not
  drawn. `Config::ternary_geometry` gives the triangle, and
  `Config::ternary_title_placement` places the axis titles along the edges.
  `Config::validate_ternary` holds the axis rules (linear, not inverted, not
  categorical, `min` 0 and the same `max`), excludes polar charts,
  overlays, line offsets, styles other than Precise and subplot links;
  `Config::validate_ternary_series` holds the series rules (only ternary
  series on a ternary chart, three distinct columns, markers or a line, a
  linear line with no fill). Edges drag along their outward normal
  (`line_offset`), tick labels in the edge frame, titles in screen space,
  and the slanted parts hit on their drawn shape. `step_selection` walks the
  points up to the shortest of the three columns. `ternary` is omitted when
  `None`, and `scatter` / `line` when unset. Migration: a `Config { .. }`
  literal adds `ternary: None`; an exhaustive `DataRenderType` match adds a
  `Ternary` arm.
- **Range bars** ([#54](https://github.com/shim9610/figgy/issues/54),
  [#52](https://github.com/shim9610/figgy/issues/52), #58 item 23).
  `DataRenderType::RangeBars` draws a bar from a start to an end value at a
  position: waterfalls, candlesticks, state timelines and Gantt charts and
  floating ranges in one kind. `x_column` holds the positions, `y_column`
  the ends and `start` the starts; `RangeWhiskers` adds a low-to-high
  stroke, `RangeColor` colours by the series, the direction of the given
  ends (with a `RangeTotal` flag column) or a state index into
  `RangeStateStyle`s, `RangeProgress` shades the unfinished share,
  `RangeConnector` joins each end to the next bar and `slot_width` sizes a
  slot on a numeric (time) position axis. The host supplies every value; the
  renderer computes no total, change or duration. `validate_range_bar_series`
  and `Config::validate_range_bars` hold the rules: a grouped layer of its
  own, a categorical or linear numeric position axis (`slot_width` exactly
  on a numeric one), a numeric value axis, linear or logarithmic, a
  connector only on a series alone in its layer. `step_selection` walks the
  rows and the range series of a layer. Every option is omitted from a
  document when unset. Labels reuse `CategoryBarSeriesLabels`, one host
  string per row.
- **Streamlines** ([#51](https://github.com/shim9610/figgy/issues/51), #58
  item 22b). `DataRenderType::Streamlines { line, arrows, color_map }`
  declares host-integrated field lines: every streamline of the series in
  its `x_column` / `y_column`, one after another, separated by a NaN row.
  `arrows: Option<StreamlineArrows { spacing_px, head_length_px,
  head_width_px }>` puts an arrowhead every `spacing_px` along the drawn
  path; `color_map: Option<PointColorMap>` colours the line and the heads by
  a host column through `Config.colorbar`. The host integrates the field and
  supplies any magnitude: the renderer computes neither. Both options are
  skipped when `None`, so the variant is additive on the wire.
  `validate_streamline_series` holds a series to a linear line without a
  fill and finite arrows with `spacing_px >= head_length_px`;
  `DataRenderType::streamlines()` borrows the fields. `step_selection`
  walks the vertices, NaN separators included; the legend shows an arrow in
  the line colour; `ColorCycle` recolours the line. Migration: an exhaustive
  `match` on `DataRenderType` needs a `Streamlines` arm.
- **`layout::auto_layout`** ([#44](https://github.com/shim9610/figgy/issues/44),
  #58 item 18). `auto_layout(&cfg, &measure) -> Config` returns the SSOT with
  axis bands, axis title offsets and tick-label formats chosen so its tick
  labels, axis titles, chart title, legend and colourbar do not overlap, as
  `measure` lays them out (the labels the renderer draws, after pruning). It
  is an explicit call, the shape of `fit_chart_title_margin`: pure,
  idempotent, and it writes only display parameters, never a range, a tick
  setting, a series, an annotation or a pick. A `Decimal` or `Plain`,
  non-categorical axis whose labels print more than five digits (`0.00300`)
  or are wider than five ems switches to `LabelFormat::Power`; each band's
  `out_margin` is set to exactly what it holds, smaller or larger
  ([#270](https://github.com/shim9610/figgy/issues/270)): the labels' reach,
  the title gap, the title's thickness and the edge padding, or the
  padding alone on a side that holds nothing, where the labels include the
  side's category group headers and the end labels of the perpendicular
  axes that run past the plot; a title that does not
  clear its labels by the gap inside its band gets the `offset_y` that
  puts it the gap past them, and one that already clears, centred or
  offset by the host, stays; overlay axes and the colourbar's axis follow the
  same rule, the chart title band is the title's height and 0.3 em of its
  font (at least the edge padding) above and below it, 0 without a title, a
  legend over the colourbar goes to the first corner that clears it, and
  each subplot is laid out as its own chart, then lined up with the cells
  of its grid row and column. No serialized field is added.
  Not covered: overlay axes drawn over each other, polar angle labels against
  radius labels, and labels that collide along an axis or at a corner. Call
  it again after an auto-fit or a zoom: the range helpers reset a linear axis
  to `Decimal`.
- **`auto_layout` fits bands both ways**
  ([#270](https://github.com/shim9610/figgy/issues/270)). Bands shrink as
  well as grow, end half the font of what sits at their outer edge (the
  title, else the labels, else beside a drawn frame the axis' labels; at
  least 8 px) past what they hold, with an axis or colourbar title half its
  labels' font (at least `LABEL_GAP`) past them, and hold the overhang of the
  perpendicular end labels and the category group headers; the chart title
  band is padded by 0.3 em above and below, 0 without a title; the plots
  of subplot cells that share a grid row or column (same chart-area y and
  height, or x and width) are lined up and sized alike, the titles of
  widened bands placed beside their labels again; a legend the host placed
  past the plot keeps its corner and offsets, each band it crosses grows to
  hold it and the edge padding of its font, and that side's axis title moves
  past it, the title gap from the legend, when beside its labels it would
  overlap it. Chart titles sit on a
  baseline: the new `MeasureText::font_line(family, size)` (by default a
  reference measurement, in the renderer the face's own line metrics) gives
  the font's line, which the placement centres in the band with the ink
  where it reaches further, so titles of one font and size share a baseline
  whatever their letters; `auto_layout` fits the band to that line, gives a
  grid row's titles one band and the `chart_title.offset_y` that puts them
  on one baseline, and `fit_chart_title_margin` (`with_title`, `set_title`)
  grows a band lower than the line. Migration: `layout::chart_title_placement`
  and `chart_title_placement_in` take a `MeasuredTitle`
  (`MeasuredTitle::of(&text, measure)`) instead of the title's
  `TextExtents`.
- **One placement per element, interchangeable sinks**
  ([#38](https://github.com/shim9610/figgy/issues/38),
  [#46](https://github.com/shim9610/figgy/issues/46) item 12). Every chart
  element has one placement function, its `emit`, which hands the parts it
  draws to a `Sink` (`model::sink`): the renderer's canvas sink paints them,
  the model's sinks answer bounds, hit tests and selection boxes from the
  same parts, and drag containment reads the same parts. There is no second
  placement left for the 15 cases the inventory listed; the ones that needed
  a shader change are done by the renderer entries below (histogram
  selection outline, contour label box, radial inside-label lift, pick slot
  keys, the picker's projection), and no `TODO(#38)` is left. The renderer
  and browser sides are under [Renderer](#unreleased-renderer) and
  [Browser product](#unreleased-web). Migration:
  - `Selectable` requires `emit(&self, cfg, measure, sink) -> bool` instead of
    `bounds`; `bounds`, `contains` and `selection_box` are provided by the
    blanket trait `SelectionGeometry` (import it next to `Selectable`) and
    cannot be overridden. `MeasureText::drawn_parts` is replaced by
    `MeasureText::emit(cfg, element, sink)`; `DrawnPart` moved to
    `model::sink` (still re-exported from `layout`) and gained `Rect` and
    `Triangle`.
  - `SelectionBox` has private fields: read `rect()`, `handles()`, `color()`,
    `stroke_width()`, `outline()`; `SelectionOutline::TickLabels(side)` is
    `SelectionOutline::Parts(Vec<DrawnPart>)`, which polar tick labels, the
    polar ray and annotations use (an arrow's shaft is outlined as its thin
    oriented box).
  - `Config::nudge` / `nudge_from` / `nudge_x` / `nudge_y`, `Draggable::drag_by`
    / `drag_from`, `Resizable::resize_by` and `DragConfig::nudge` / `nudge_at`
    take a `&dyn MeasureText`. A move is contained on the drawn parts: it is
    rejected unless one of the element's parts keeps its centre inside the
    chart area, and when the measure cannot place the element or it draws
    nothing. `Resizable::resize_handles` reports its handles through a
    callback.
  - `AnnotationElement.text_size` is `text: Option<MeasuredText>`;
    `AnnotationShape`, `annotation_shapes`, `shapes`, `label_rect` and
    `geometry_bounds` are gone (`emit_annotation` and
    `AnnotationElement::emit` place the parts). An annotation's bounds are its
    drawn parts without grab slack; it hits inside the outlines its selection
    draws (`part_outline`: a stroke's box is half its width plus 2 px, at
    least 3 px, either side (`segment_grab_half_width`) and 2 px beyond the
    ends; a triangle grows 2 px outward (`padded_triangle`); a rectangle
    2 px). `ARROW_LABEL_GAP` and `LINE_LABEL_INSET` (pixels) are gone: the
    label gaps are `ARROW_LABEL_GAP_EM` and `LINE_LABEL_INSET_EM`, relative
    to the label font, so they scale with the display scale.
  - Every element hits inside its padded selection box (2 px outside its
    bounds): the data area, the legend, the colourbar strip, the chart and
    axis titles and every bounded chart decoration, like the axes and tick
    labels did before. An element that outlines its parts hits inside each
    part's outline (`select::part_outline`, the one shape of the hit test,
    the canvas highlight and a host's outline): a text block or rectangle as
    its box grown by 2 px, a stroke as the box `max(width / 2 + 2, 3)` px
    either side of its line and 2 px beyond its ends (the plan said
    `max(width / 2, 3)`; the polar ray is grabbed within 3 px, was 4), an
    arrow head as its triangle grown 2 px outward. A text annotation is one
    box.
  - A layered document expands its base panel (`select::on_drawn_panel`,
    the renderer's own expansion, SSOT configuration) for every placement
    the model runs for a hit test or a drag step; a single-layer document
    copies nothing.
  - Removed: `LineInterpolation::piece_count`,
    `PolarInterpolation::piece_count`, `layout::axis_visibility_extent` /
    `axis_visibility_rect` / `rect_axis_visibility_rect`, which nothing in
    the workspace called any more; an axis's or tick band's drawn box is its
    element's `SelectionGeometry::bounds`. Added:
    `model::sink` (`Sink`, `Slot`, `PlacedText`, `DrawnPart` and the types
    they carry),
    `select::PartOutline`, `part_outline`, `segment_grab_half_width`,
    `padded_triangle`, `on_drawn_panel`, `layout::axis_fraction`,
    `span_fraction`, `guarded_log_range` (moved from the renderer),
    `visible_colorbar_rect`, `Config::title_band`.
- **Character sets.** `RichSegment.greek: bool` becomes
  `char_set: CharSet` (`Text`, `Greek`, `Math`), so a segment cannot be
  Greek and math at once
  ([#45](https://github.com/shim9610/figgy/issues/45)). The JSON is
  unchanged for every existing document: `Text` writes `"greek": false`,
  `Greek` writes `"greek": true`, and `Math` writes `"greek": false`
  followed by the new `"math": true`, which is written only for `Math`.
  Documents written by model 0.12 and earlier parse and re-serialize byte
  for byte; a segment with both `"greek": true` and `"math": true` is
  rejected with an error. `Math` draws the character as given from the
  bundled math face (see the math layout below). Model 0.12.1 and older
  ignore `math` and read such a segment as text. The browser
  product's `get_config` / `set_config`, `get_series` / `set_series`
  and annotation calls carry the new key. Migration: write
  `char_set: CharSet::Text` for `greek: false` (or start from
  `RichSegment::plain`), `char_set: CharSet::Greek` for `greek: true`, and
  `seg.char_set == CharSet::Greek` for reads of `seg.greek`; `CharSet` is
  `model::text::CharSet` (`renderer::text::CharSet`), and
  `CharSet::glyph_char` gives the character a set draws.
- **Math nodes and `RichText::from_tex`**
  ([#45](https://github.com/shim9610/figgy/issues/45), #58 item 19). A
  formula is a tree over the flat segment list, encoded by two additive
  `RichSegment` fields: `math_node: Option<MathNode { id, kind }>` on a
  segment that governs a layout and `math_parent: Option<MathSlot { id,
  slot }>` on every segment inside one. A node comes first and its
  descendants follow it contiguously, slot by slot. `MathKind` is
  `Math(MathStyle)` (a root: math mode, `MathStyle::Text` by default or
  `MathStyle::Display`, written `{"Math":"Text"}` / `{"Math":"Display"}`),
  `Frac` / `Binom` (`FracStyle::{Auto, Display,
  Text}`), `Sqrt`, `Scripts` (also a text-mode root), `BigOp` / `OpName`
  (`Limits::{Auto, Limits, NoLimits}`), `Delim { open, close }`, `Middle`,
  `Sized { level, class }`, `Accent` (`AccentKind::{Fixed, Wide}`) and
  `Over` / `Under` (`OverUnder::{Line, Brace, Arrow, Set}`), all in
  `model::text::math` and re-exported from `model::text`. `MathCursor`
  walks a tree with a fixed stack of `MAX_MATH_DEPTH = 16` open nodes and
  `RichText::validate_math` reports a `MathError { at, kind }` for twelve
  kinds (duplicate or out-of-order ids, an unknown or closed parent, a slot
  out of range or order, nesting past 16, a node outside `Math`, nested
  `Math`, `Middle` outside `Delim`, a break inside math, a script flag
  inside a node, a bad delimiter). Ids increase within a cell and are
  unique only there, so two legend labels that each start at id 0 stay
  valid. `math_class` (Unicode `MathClass.txt`) and `math_alphanumeric`
  (the bold, italic and bold-italic alphabets) are the tables the layout
  reads. `RichText::from_tex(tex, color, font_size, font)` and
  `rich_segments_from_tex` convert text with `$…$` / `\(…\)` math spans
  (text style) and `$$…$$` spans (display style; a leading `\displaystyle`
  or `\textstyle` sets a span's style) in the TeX subset of #45 (`model::text::tex`; errors are `TexError { at,
  kind }` with the byte range); the source is not stored. A negative
  `field_em` (`\!`) is allowed on a space inside `Math` only. The JSON is
  additive: both keys are written last and only when set, so every older
  document re-serializes byte for byte, and `RichText` deserializes through
  a mirror that rejects a malformed tree at parse. Annotation texts,
  bounded titles and the radial title validate the tree. Model 0.12.1 and
  older ignore both keys and draw a formula's characters flat. Migration:
  a `RichSegment` struct literal that lists every field adds
  `math_node: None, math_parent: None`; literals built with
  `..RichSegment::plain(c)` need no change.
- **Plain number labels.** `LabelFormat::Plain(PlainNumberFormat {
  decimals, trim_zeros })` writes a value alone, ignoring scale, spacing
  and significant digits: `decimals: None` (`PlainNumberFormat::AUTO`) is
  the bounded value axes' tick form (`2.5`, `1000`, `1.23e6`), `Some(d)`
  exactly `d` decimals, trailing zeros trimmed with `trim_zeros`. The
  categorical, radial, box plot and radar data labels and the value-axis
  tick labels of the categorical, box plot and radar charts use it, with
  the same text. The JSON is additive (`{"Plain":{}}`, both fields omitted at
  their defaults); model 0.12.1 and older reject it. Migration: a Rust
  `match` over `LabelFormat` adds a `LabelFormat::Plain(_)` arm.
- **One tick plan.** The Cartesian spacing rule (`Chart::set_x_range`,
  `set_y_range` and the fits), `AxisOptions::compute_nice_ticks`, the tick
  marks, labels and grid lines of Cartesian axes and the colourbar, the
  radar rings, the box plot and categorical value ticks and the contour
  levels of `ContourConfig::nice_levels` now share one spacing rule and
  one walk in `model::tick` (`renderer::tick`)
  ([#58](https://github.com/shim9610/figgy/issues/58) item 14). New public
  items: `AXIS_LINEAR_TARGET` (10), `AXIS_LOG_TARGET` (7), `LooseRange`,
  `nice_linear_step`, `nice_log_step`, `axis_major_spacing`, the walkers
  `LinearGrid`, `LogDecades`, `LogMinors`, `major_ticks`, `minor_ticks` and
  their iterator types, the guards `walkable_range`,
  `range_in_spacing_units`, `spacing_advances_range`,
  `automatic_linear_spacing`, `effective_major_spacing`,
  `major_spacing_rejection`, `drawn_minor_count` and
  `effective_log_decade_step`, and the walk limits. Migration:
  `SpacingRejection` moves to `model::tick` and stays exported as
  `renderer::SpacingRejection`. Output changes:
  1. a linear spacing that `set_x_range` / `set_y_range` or a fit writes
     can differ in its last bits (at most 2 ulp; `2·10^(k-1)` where it was
     `0.2·10^k`), and a step below `f64::MIN_POSITIVE` can be rounded
     differently. Tick labels are unchanged except Decimal labels of
     values past 2^53, which print every integer digit, and of steps below
     1e-15, where the 15-decimal cap already merges neighbouring labels;
  2. a span above about 1e308 keeps its 1·2·5 step class instead of
     falling back after the loose range overflowed (`compute_nice_ticks`
     of `0..1.05e308` at target 5 steps `5e307`, not `2e307`;
     `set_x_range` steps `2e307`, as below the overflow);
  3. radar rings and box plot value ticks keep a ring or tick that
     rounding put just past a range end, through the walk's 1e-9 step
     slack (`value_range [0, 0.3]` with `ring_spacing 0.1` draws the 0.3
     ring). Fitted ranges gain it most: in a seeded sample, 0.7 % of
     fitted radar axes gain the outer ring and 7 % of fitted box plot axes
     gain an end tick; fixed ranges rarely change;
  4. categorical value ticks are `k · step` instead of `min + i · step`:
     about one value in ten moves by an ulp, and a label changes only on a
     rounding tie of the exponent form (28 of 525,313 in the sample);
  5. the log decades of Cartesian axes, grid lines and the colourbar are
     computed with `powf` instead of `powi`, as contour levels already
     were: a decade value can move by up to 7 ulp at large exponents, and
     an axis reaching subnormal decades gains the ticks `powi` lost;
  6. `nice_levels` returns the level-limit error for ranges whose level
     indices passed `i64` (the walk saturated and returned no levels), and
     no levels for a step that moves neither range end at `f64` precision
     (the walk repeated the few representable values).
- **One material enum.** Bars, boxes and sectors take one `Material`
  (`Flat`, `Matte`, `Ceramic`, `BrushedMetal`, `Paper`, `Wood`,
  `SatinMetal`, `Toon`, `Enamel`, `Hatch`, `Pearl`), and a name has one
  look in every family: one shader function draws all three
  ([#46](https://github.com/shim9610/figgy/issues/46)).
  - Bars and boxes are shaded like a flat pie top under the default light,
    with uniform lighting and highlights. **The look of Matte, SatinMetal,
    Enamel and Paper bars and of Matte and SatinMetal boxes changes**: no
    vertical gradient (Matte) and no highlight band (SatinMetal, Enamel).
    Boxes default to Matte, so a default box plot changes too. `gloss` has
    no visible effect on an Enamel bar, and at 1x a `texture_scale` above
    0.13 none on a metal bar.
  - Bars and boxes gain `Ceramic`, `BrushedMetal`, `Wood` and `Hatch`
    (boxes also `Enamel` and `Paper`). `Toon` and `Pearl` need a curved
    surface: `Material::supported_on_flat_body` is false for them, and bar
    and box validation rejects them ("material not supported by bars" /
    "material not supported by boxes"). Sectors accept every material.
  - Per-family defaults are unchanged (bars Flat, boxes Matte, sectors
    Flat). Serialized documents are unchanged: the variant names are the
    strings the three old enums wrote.
  - Migration: `RadialMaterial`, `CategoryBarMaterial` and `BoxPlotMaterial`
    are deprecated aliases of `Material`; replace the name, the variants are
    the same. `BoxPlotMaterial::default()` is now `Flat` (the enum default);
    `BoxPlotStyle::default()` still uses `Matte`. An exhaustive `match` on
    `CategoryBarMaterial` or `BoxPlotMaterial` needs the new variants.
- **The polar label band scales with the chart**
  ([#47](https://github.com/shim9610/figgy/issues/47)). `Config::polar_geometry`
  reserved a fixed 6 px beside the tick length and the label line (2 px
  beside the tick length with the angle labels hidden), which
  `Config::scaled` could not scale: a 2x
  display or PNG export of a polar chart put the ring, its ticks and both
  label sets 2.5 to 4 px (1x) off the 1x layout doubled. The clearance is now
  a third of the angle-label font size (a ninth with the labels hidden), so
  2x is the 1x layout doubled. **Behaviour change:** at the default 18 px
  label font the layout is unchanged to the bit; a polar chart with another
  `bottom_x.label_style.font_size` changes its ring radius at 1x by
  `(18 - font_size) / 3` px (`(18 - font_size) / 9` px with the labels
  hidden), for example 2 px larger at 12 px and 1 px smaller at 21 px. No API
  or serialized field changes.

- **The bounded group, classified per chart**
  ([#40](https://github.com/shim9610/figgy/issues/40) item 13, 13a to 13i).
  The four charts the 0.12 releases grouped as "bounded" are classified on
  their own. The radial chart is the DataLess chart, defined by its SSOT
  alone. Category bars and box plots are Cartesian series kinds and radar
  series a polar one, their per-point data entering through `ColumnSource`.
  `CategoricalChart`, `BoxPlotChart` and `RadarChart` stay only as
  deprecated readers of the documents of releases up to 0.12. Released
  fixtures written by model 0.12.1 pin every document (13a). Every migration
  of the item is under
  [Migrating from the bounded group](#unreleased-bounded-migration).
  - **The radial chart is the DataLess chart**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13b). The model
    documents two chart classes: a DataLess chart is defined by its SSOT alone,
    with no `ColumnSource`, no column pool and no streaming, and the radial
    chart is its only member; the series kinds of `Config` are the
    column-backed class. No new type.
  - **Radial label strings in the SSOT.** `RadialSlice` gains `text:
    Option<String>` (additive, omitted when absent): the drawn label, at most
    two lines joined by `'\n'`, each at most 400 characters without control
    characters. `RadialChart::format_labels()` writes every slice's and detail
    child's text from the label format, `label_decimals`, `value_suffix` and its
    circle's total at edit time; `normalize_labels()` fills only the missing
    ones, so a document written before the field draws the strings it drew;
    `labels_have_text()` says whether a frame can draw every shown label.
    `label_lines` is the helper both use.
  - **Radial title through the shared chart title.** `RadialChart.chart_title`
    is `Option<ChartTitleOptions>` instead of `Option<BoundedTitleOptions>`,
    written and read as the same `{"visible", "text"}` object in the same order,
    so documents keep their bytes. The position stays in `decorations`: the
    offsets and `top_margin` read as 0 and `RadialChart::validate` rejects other
    values. `resolved_title()` returns a `ChartTitleOptions` built by the new
    `ChartTitleOptions::from_legacy_plain(title, font, label_size, color)`.
    `select::emit_chart_title_in(title, chart_area, top_margin, measure, sink)`
    is the one title placement, split out of the Cartesian title (whose pixels
    do not change), with `layout::chart_title_placement_in` its `RectF`
    arithmetic. The rich-text limits of titles and annotations are one
    check, `RichText::within_limits()`. `CategoricalChart`, `BoxPlotChart` and
    `RadarChart` keep `BoundedTitleOptions`.
  - **`RadialChart` is a drag state.** `impl DragConfig for RadialChart`
    (`Element = DecorationElement`): a decoration of the radial frame's
    `HitMap<RadialChart>` moves in `decorations` by `DecorationLayout`'s rule.
    `SelectionBox::clipped_to(area)` intersects a selection box with a canvas.
    `RadialChart::step_selection(backwards)` is the radial step of the web
    wrapper's `step_selection`, moved into the model since it edits only the
    SSOT; the wrapper calls it and its behaviour is unchanged.
  - **Categorical axes** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13c). `AxisOptions` gains `categories: Option<Vec<Category>>`; a
    categorical axis with `n` categories maps the data range `[-0.5, n - 0.5]`
    with category `i` at `i`, a major tick at every category, no minors and
    `categories[i].label` as tick text, thinned by the numeric labels' stride
    rule when they crowd. `min`, `max`, `major_spacing`, `minor_count` and the
    label format are ignored on it. New items: `AxisOptions::category_count`,
    `data_range` (what the tick walks, `axis_fraction`, the annotation space and
    the overlay and subplot expansion read) and `validate_categories` (1 to 64
    categories, unique non-empty single-line ids, single-line labels of at most
    80 characters, `MAX_CATEGORY_TEXT_CHARS`, a linear scale; an inverted
    categorical axis walks its categories from the far end, the ticks, labels,
    grid, data and picks following the same `axis_fraction`);
    `PolarOptions::validate` rejects a categorical angle axis (it arrives with
    the radar series kind) and radius axis. A linked subplot axis follows its
    target's categories. `Category` moves to `model::config` beside the axis;
    `categorical::Category` (and the crate-root path) re-export it. The JSON key
    is additive (omitted when `None`); model 0.12.1 and older ignore it.
  - **Axis anchors take the plot rectangle.** `layout::axis_anchor(side, plot:
    &RectF)` replaces `axis_anchor(side, &DataArea)`, so a plot that is not
    integer-aligned is anchored by the same code; an integer data area gives the
    same endpoints bit for bit.
  - **Categorical bars are a Cartesian series kind**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13d).
    `DataRenderType::CategoryBars { color, style, labels, label_format,
    overrides }` draws one bar per category of the chart's categorical axis
    (`bottom_x` for vertical bars, `left_y` for horizontal ones): the positions
    column holds `0..n-1`, the values column one value per category (NaN for a
    missing bar). The category bar series of one layer form its bar group, laid
    out by the new `Config.bar_groups: Option<BarGroups>` (`mode` `Grouped` /
    `Stacked` / `PercentStacked`, `direction`, `group_width`, `bar_gap`; `None`
    reads as the default). `CategoryBarMode`, `CategoryBarDirection`,
    `CategoryBarLabels`, `CategoryBarLabelFormat`, `CategoryBarStyle`,
    `CategoryBarTarget` and `CategoryBarOverride` move to `model::data_config`;
    `model::categorical` re-exports them, so existing paths keep working. New
    items: `BarGroups`, `CategoryBarSeriesLabels` (placement, decimals, suffix
    and `text`, one drawn string per category), `CategoryBarsRef` /
    `DataRenderType::category_bars`, `SeriesConfig::category_bar_columns`,
    `validate_category_bar_series`, `Config::validate_category_bars` (a
    categorical axis in the bar direction, a linear value axis, at most
    `MAX_BAR_SERIES` series per group and `MAX_CATEGORY_BARS` bars, distinct
    value columns, no polar chart, horizontal groups on the base panel only),
    `Config::bar_groups_of` and `format_labels`, which writes `labels.text` from
    the host's values at edit time. The renderer draws the text it is given and
    never formats a value (value labels, #40 item 13h, below). `AxisOptions`
    gains `cross_at: Option<f64>`: the value of the perpendicular axis the axis
    line crosses at (`layout::axis_cross`; `axis_anchor(side, plot, cross)`
    takes it); only the line moves, the labels and the title stay at the plot
    edge. The overlay panels of `Config::expand` keep their categorical axes'
    categories and carry `bar_groups`. Every new key is additive and omitted at
    its default; model 0.12.1 and older documents parse and re-serialize byte
    for byte.
  - **The categorical chart is a document reader**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13d).
    `CategoricalChart` stays the serialized document of releases up to 0.12;
    `Config::from_categorical(&chart) -> (Config, Vec<SeriesConfig>,
    Vec<SeriesUpload<'_>>)` (`model::bounded_sources`) turns it into a
    Cartesian chart with a categorical axis, `bar_groups` and one
    `CategoryBars` series per document series. The uploads borrow the
    document's values (`UploadValues`, a `ColumnSource` in the renderer):
    the positions column `categorical.positions` first, then one value
    column `categorical.values.<series id>` per series; nothing is copied.
    The titles, grid, legend, font, colours, chart title, annotations,
    active layer, selection and decoration offsets map onto their Cartesian
    options; the hover state, the background colour, the explicit
    `decorations.data_area` and the data label offsets are not carried. A
    horizontal document's category axis is inverted, so its first category
    stays at the top and its `Y` annotations land on the same categories.
    Removed: `CategoricalChart::segments`, `CategoryBarSegment` and
    `CategoricalChart::label_text` (a bar's geometry is the shader's, its
    label `format_labels`').
  - **Box plots are a Cartesian series kind**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13f).
    `DataRenderType::BoxPlot { color, style, labels, outliers, overrides }`
    draws one box per category of the chart's categorical axis (`bottom_x` for
    vertical boxes, `left_y` for horizontal ones) with its median, whiskers and
    caps, mean and outliers. `x_column` is the positions column `0..n-1`,
    `y_column` the id of the series' record of box statistics (the renderer's
    `BOX_PLOT_RECORD` schema: `median`, `q1`, `q3`, `whisker_low`,
    `whisker_high` and the optional `mean`, `sample_count`, `ci_low`, `ci_high`;
    a missing box is a row of NaN), `outliers` an optional ragged column with
    one group per category; the roles do not swap with the direction. The box
    plot series of one layer form its box group, laid out by `Config.bar_groups`
    (`direction`, `group_width`, `bar_gap` as the box gap; the mode must be
    `Grouped`). `BoxPlotTarget`, `BoxPlotLabels`, `BoxOutlierShape`,
    `BoxPlotStyle` and `BoxPlotOverride` move to `model::data_config`;
    `model::boxplot` re-exports them, so existing paths keep working. New items:
    `BoxPlotSeriesLabels` (placement, decimals, suffix and `text`, one drawn
    string per category, at most `MAX_BOX_PLOT_LABEL_CHARS` each), `BoxPlotRef`
    / `DataRenderType::box_plot`, `SeriesConfig::box_plot_columns`,
    `BoxPlotStyle::validate`, `validate_box_plot_series`,
    `Config::validate_box_plots` (a categorical axis in the box direction, a
    linear value axis, at most 16 box plot series per layer and 512 boxes, a
    grouped mode, one kind per layer: a layer draws either category bars or box
    plots, no horizontal boxes on an overlay, no polar chart) and
    `format_box_plot_labels`, which writes `labels.text` from the host's medians
    and counts at edit time: the renderer has no per-box statistics to write it
    from, so a host that uploads its own records writes it. The renderer draws
    the text it is given (value labels, #40 item 13h, below). Every new key is
    additive and omitted at its default; model 0.12.1 and older documents parse
    and re-serialize byte for byte.
  - **The box plot is a document reader**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13f).
    `BoxPlotChart` stays the serialized document of releases up to 0.12 and
    keeps its `validate` (ordered statistics, outliers outside the whiskers, a
    CI around the median, the limits). `Config::from_boxplot(&chart) -> (Config,
    Vec<SeriesConfig>, Vec<SeriesUpload<'_>>)` (`model::bounded_sources`) turns
    it into a Cartesian chart with a categorical axis (top-down for horizontal
    boxes, as before), a grouped `bar_groups` and one `BoxPlot` series per
    document series. The uploads borrow the document's summaries: the positions
    column `boxplot.positions` (`BOX_PLOT_POSITIONS_COLUMN`) first, then per
    series its record `boxplot.record.<series id>` (`UploadValues::BoxRecord`,
    whose nine field adapters `UploadValues::box_fields` gives as `BoxColumn`s)
    and, when any of its boxes has an outlier, its ragged column
    `boxplot.outliers.<series id>` (`UploadValues::BoxOutliers`, values and
    offsets from `UploadValues::box_outliers`); every value is computed from the
    summaries on the fly, nothing is copied. `value_range` becomes the fixed
    value axis; the titles, grid, legend, font, colours, chart title,
    annotations, active layer, selection, label settings (with the text written
    from the medians and counts), overrides and decoration offsets map onto
    their Cartesian options; the hover state, the background colour, an explicit
    `decorations.data_area` and the data label offsets are not carried. Removed:
    `BoxPlotChart::label_text` (a box's label is `format_box_plot_labels`').
  - **Categorical angle axis and polygon grid**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13g). A polar
    chart's angle axis (`bottom_x`) may now be categorical
    (`AxisOptions::categories`), which `Config::validate_polar` rejected before:
    category `i` of `n` sits at `i / n` of the turn (or of `sector`) from
    `zero_angle_degrees`, in the `clockwise` direction, so the angle column
    holds the category positions `0..n`. `polar::AngleMap`
    (`Config::polar_angle_map`, `PolarOptions::angle_map`) is the one mapping
    from an angle value to the screen for both axis kinds; on a numeric axis it
    gives the constants the projection used before, bit for bit.
    `PolarOptions.grid_shape: Option<GridShape>` (`Circle`, the default, or
    `Polygon`) joins every ring and the outer ring with chords at the
    categories; a `Polygon` grid needs a categorical angle axis (`InvalidConfig
    { field: "polar" }` otherwise). The key is additive and omitted at its
    default.
  - **Radar is a polar series kind**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13g).
    `DataRenderType::Radar { color, style, labels }` draws a closed outline
    through one value per category of a polar chart's categorical angle axis:
    `x_column` the positions column `0..n`, `y_column` the values. The closing
    segment joins the last category to the first; a NaN value breaks the outline
    and the fill there. `RadarSeriesStyle`, `RadarMarker` and `RadarLabels` move
    to `model::data_config`; `model::radar` re-exports them, so existing paths
    keep working. `RadarSeriesStyle` gains the optional `fill_opacity` (the
    default fill is the colour at 25 %), omitted when unset, and the `DEFAULT_*`
    constants, `resolved_from(&RadarStyle)` and `validate`. New items:
    `RadarSeriesLabels` (placement, decimals, suffix and `text`, one drawn
    string per category, at most `MAX_RADAR_LABEL_CHARS` each), `RadarRef` /
    `DataRenderType::radar` (with the line and marker styles the series draws
    with), `validate_radar_series`, `Config::validate_radar` (a polar chart with
    a categorical angle axis and `Chord` interpolation, at most 16 radar series
    per layer) and `format_radar_labels`, which writes `labels.text` at edit
    time. The renderer draws the text it is given (value labels, #40 item 13h,
    below). Every new key is additive and omitted at its default; model 0.12.1
    and older documents parse and re-serialize byte for byte.
  - **The radar chart is a document reader**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13g). `RadarChart`
    stays the serialized document of releases up to 0.12 and keeps its
    `validate`. `Config::from_radar(&chart) -> (Config, Vec<SeriesConfig>,
    Vec<SeriesUpload<'_>>)` (`model::bounded_sources`) turns it into a polar
    chart whose angle axis holds the categories and one `Radar` series per
    document series, each with its style's unset fields read from the chart
    style. The uploads borrow the document's values: the positions column
    `radar.positions` (`RADAR_POSITIONS_COLUMN`) first, then one value column
    per series, `radar.values.<series id>` (`radar_values_column`,
    `UploadValues::Dense`); nothing is copied. `start_angle_degrees` becomes
    `zero_angle_degrees` (negated: the document measured with y down) and the
    ring labels' ray, `clockwise` and `grid_shape` the polar options,
    `value_range` the fixed radius axis with `ring_spacing` as its spacing
    (without a `value_range` the range is the host's to fit); the grid, ring and
    spoke colours, font, colours, legend, chart title, annotations, active
    layer, a selected vertex, label settings (with the text written from the
    values) and the title, legend and ring label offsets map onto their polar
    options; the hover state, a selected whole series, the chart style's
    `emphasis_brightness`, the background colour, an explicit
    `decorations.data_area`, the spoke and value label offsets and the
    decoration selection are not carried. Removed: `RadarChart::label_text` (a
    vertex's label is `format_radar_labels`').
  - **Value label offsets, the label hit id and the label selection**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13h). The value
    labels of `CategoryBars`, `BoxPlot` and `Radar` series are drawn (see
    [Renderer](#unreleased-renderer)); their strings stay `labels.text`, written
    at edit time. `CategoryBarSeriesLabels`, `BoxPlotSeriesLabels` and
    `RadarSeriesLabels` gain `offsets: Vec<[f32; 2]>`: the drag offset of each
    label in logical pixels (x right, y down), one per category, empty (and
    omitted from JSON) until a label is dragged; validation takes it empty or
    one finite offset per category of at most `MAX_VALUE_LABEL_OFFSET` (4096) px
    along either axis. `SeriesConfig::drag_value_label(index, categories, dx,
    dy)` writes it (the first drag fills one zero offset per category; a move
    past the bound, a non-finite delta or an index past the categories is
    rejected unchanged), with `value_label_offsets`, `value_label_offset` and
    `value_label_text`. `value_label_id(series_id, index)` /
    `parse_value_label_id` name a label `"series:<series_id>:label:<index>"` for
    hosts. `PickedDataRef::ValueLabel { source_id, series_id, category_index }`
    is a selected label (`{"kind":"value_label", …}` in `picked_data`). The
    readers carry a legacy document's `DataLabel` offsets into `labels.offsets`.
  - **Readers deprecated, their dead helpers removed** (13i).
    `CategoricalChart`, `BoxPlotChart` and `RadarChart` are
    `#[deprecated(since = "0.13.0")]`, each message naming its reader
    (`Config::from_categorical`, `Config::from_boxplot`, `Config::from_radar`);
    they are removed in the next milestone. The helpers of the removed
    renderers and editors that no reader needs are gone:
    `CategoricalChart::bar_override`, the three types' `reorder_categories`,
    `set_active_layer` and `normalize_title`, and `RadarChart::is_emphasized`
    and `resolved_style`; `active_layer()`, `resolved_title()`, `validate()`
    and the lookups `validate` uses stay. `DecorationElement.followers` (the
    followers of the removed bounded axes, always empty on the radial chart)
    is removed with the follower check of `DecorationLayout`'s drag rule.
    Kept, because the radial chart or a reader needs them: `DecorationLayout`,
    `DecorationOffset`, `DecorationElement`, every `DecorationId` variant
    (documents carry them), `BoundedTitleOptions` (the documents'
    `chart_title`) and the `bounded_title` and `bounded_sources` modules. The
    model README names the two chart classes and the readers.
  - **Cartesian selection stepping in the model** (13i).
    `Config::step_selection(series, dx, dy, column_len) -> Result<bool,
    &'static str>` is the web wrapper's Cartesian `step_selection` moved into
    the model, with the same order and clamp rule: a point along its series
    and across the point-drawing series (lines, markers, radar vertices, point
    vectors), a category bar or box along its categories and across its
    layer's group (the category axis taking its pointer direction), a
    histogram bin along its bins and across the histograms of its
    orientation, a matrix cell over its grid (a grid vector's `u` grid); a
    contour level and a value label do not step. `column_len` gives the
    registered columns' lengths; it walks the series without collecting
    them.
- **Adaptive log-axis ticks**
  ([#42](https://github.com/shim9610/figgy/issues/42),
  [#58](https://github.com/shim9610/figgy/issues/58) item 20).
  `AxisOptions::log_ticks: LogTickPolicy` decides a logarithmic axis'
  majors. `Decades`, the default, is the walk every log axis took before:
  every `major_spacing`-th decade. `Adaptive` ignores `major_spacing` and
  plans from the range alone: `tick::log_ladder` returns
  `LogLadder::Decades(n)` over more than six decades (`n` is
  `tick::axis_major_spacing`), otherwise the first of whole decades,
  `LogLadder::OneTwoFive` (1, 2 and 5 × 10^k) and
  `LogLadder::AllMantissas` (1 to 9 × 10^k) that puts at least
  `tick::LOG_LADDER_MIN_MAJORS` (4) majors inside the range, else
  `AllMantissas`. `major_ticks` and `minor_ticks` walk the ladder, the minors
  being the multiples 2 to 9 that are not majors (`minor_count` 0 still
  hides them), so the strokes, grid, labels, polar rings and ray, colourbar
  and hit geometry follow it. `tick::LogMultiples::new(min, max, mask)`
  walks any set of multiples and `LogMinors` is its 2..9 set, bit for bit;
  `tick::log_multiple(value)` names the multiple of a walked value and
  `tick::log_label_ladders(axis)` lists the label ladders the renderer
  demotes along. A subplot link copies the policy with the tick spacing
  (`copy_axis_range`), the range helpers and a view state keep it, and a
  hidden panel axis takes its template's. The field is additive: only
  `Adaptive` is written, so released documents re-serialize byte for byte.
  Migration: an `AxisOptions { .. }` literal or exhaustive destructuring
  names `log_ticks` (`LogTickPolicy::Decades` keeps the former ticks).
- **Offset spectra: `Config::line_offsets`**
  ([#50](https://github.com/shim9610/figgy/issues/50),
  [#58](https://github.com/shim9610/figgy/issues/58) item 22a). The
  additive `line_offsets: Option<LineOffsets { series, step_y, step_x }>`
  draws line series stacked: member `k` (stacking order) shifted by
  `k * step_y` and `k * step_x`, in data units or decades on a logarithmic
  axis. A subplot's chart has its own field. `Config::line_offset_of(id)`
  returns a member's shift; `LineOffsets::validate`,
  `Config::validate_line_offset_layout` and
  `Config::validate_line_offsets(series)` reject non-finite steps, unknown or
  duplicate ids, non-line members (`Line`, `ScatterLine` and
  `LineScatterErrorbar*` are accepted), members another chart draws, polar
  charts, more than `MAX_OFFSET_LINES` (32) members and a `ToSeries` fill
  across members. `Config::expand` appends one panel per member after the
  overlay panels (the panel of the layer that scales it with `bottom_x` and
  the transform's `left_y` moved by the shift, every axis element hidden,
  the title, grid, legend and colourbar hidden, only the member's picks);
  the base and overlay panels draw no member. A member panel is not a layer:
  `panel_layers()` reports its owner's layer for it, `panel_member(index)`
  names the member a panel draws, `panel_of_series(id)` routes a series to
  its panel and `has_offset_members()` says whether a chart has any.
  `scale_in_place` keeps the steps. The field and `step_x` are omitted at
  their defaults, so released documents re-serialize byte for byte.
  Migration: a `Config { .. }` literal or exhaustive destructuring names
  `line_offsets` (`None` keeps the former drawing); a subplot's chart is a
  `Config` and does the same. A caller that zipped `panel_layers()` with
  `Expansion::panels` keeps working; one that assumed each layer appears
  once checks `panel_member(index)`.

- **Broken axes** ([#200](https://github.com/shim9610/figgy/issues/200)).
  `AxisOptions` gains `breaks: Option<AxisBreaks>`: ranges cut out of a
  value axis. `AxisBreaks { ranges: Vec<AxisBreak { from, to }>, style,
  gap_px, mark, frame_each_panel }` with `AxisBreakStyle::{Continuous, Panels}`
  and `BreakMark::{None, Slash { size, color: Option<Color>, width }}`
  (`None` draws in the axis line's colour); `MAX_AXIS_BREAKS`
  (3) cuts per axis and gaps up to `MAX_AXIS_BREAK_GAP_PX` (200 logical
  pixels, #249). `AxisOptions::validate_breaks` (called by `Config::validate`)
  refuses non-finite, empty, unsorted or overlapping ranges, non-positive
  ones on a log axis and an out-of-range gap; breaks on `top_x`,
  `right_y` or a colourbar axis, and on polar, ternary, 3D or parallel
  charts, are refused too, and a hexagonal grid on a `Continuous` broken
  axis. `layout::BreakMap` (`of`, `remap`, `unmap`, `pieces`,
  `raw_pieces`, `knots`, `scale`) is the piece layout every consumer
  shares: the pieces split the axis in proportion to their spans at one
  scale, each cut drawn as its gap. `layout::axis_fraction`,
  `AxisSpace` (gains `breaks`; `fraction` is `None` inside a cut, new
  `fraction_across_gaps`) and `value_in_cut` follow it, so ticks, grid
  lines and labels skip the cuts, and grid lines are drawn per piece;
  `tick::effective_major_spacing` steps a broken linear axis' spacing down
  the 1, 2, 5 × 10^k ladder while a piece would hold fewer than two majors,
  and crowded labels are thinned on the unbroken grid, so each piece keeps
  round values. The
  axis line is drawn per piece with the mark's slashes
  (`AxisStroke::Break`) and, with `frame_each_panel` (default `true`, in
  both styles), each piece's edges along the gaps (`AxisStroke::Frame`),
  all part of
  the axis element: they move with its `line_offset`, and an axis that
  frames its pieces is hit per part, so its frames do not make the plot
  select it.
  `layout::mirrored_breaks` gives `top_x` and `right_y` their partner's
  cuts at the same fractions when drawn. Spans and reference lines across
  a gap are emitted per piece. `scale_in_place` scales the mark;
  `copy_axis_range`, the hidden overlay axis and the line offset members
  carry the cuts (a member's move with its range). Migration: an
  `AxisOptions { .. }` literal or exhaustive destructuring names `breaks`
  (`None` keeps the former drawing); an `AxisSpace` literal names
  `breaks: BreakMap::IDENTITY`; an exhaustive `match` on `AxisStroke`
  adds `Break(..)` and `Frame(..)` arms.

- **Energy-level (Grotrian) diagrams** ([#203](https://github.com/shim9610/figgy/issues/203)).
  `DataRenderType::EnergyLevels(EnergyLevelSeries)` is a DataLess series:
  its `levels` (`EnergyLevel`: id, term column, NIST configuration, term,
  J, g, prefix, energy, suffix, uncertainty, reference, a `LevelRole` of
  `Level`, `Centroid` or `Component { multiplet }`, a rich label and its
  drag offset), `limits` (`IonizationLimit`) and the `transitions` the host
  lists (`Transition`: lower and upper level ids, a label, a colour value,
  a slot offset and the NIST line fields) are SSOT configuration, with
  `style`, `show` (`LevelView::Main` or `Components`) and an optional
  `source` naming the series whose records a view draws. No list has a
  count limit. `Config::validate_energy_levels` (in `validate_series`)
  needs a categorical `bottom_x` on a rectangular chart and refuses
  columns and a self source. `LevelPlacement` places and emits every part
  through the `Sink` (strokes, limit lines, arrows, labels) and picks over
  the same placement (`LevelHit`); `step_selection` walks the rows a view
  draws; `continuum_annotations` / `write_continuum` write a span per
  limit; `Config::inset_marker` writes an inset's window and connectors;
  `Config::nested_subplot_at` finds the inset under a point.
  `PickedDataRef::ValueLabel` gains `part: Option<LabelPart>`.
  `Category` gains `rich_label: Option<RichText>` (drawn and measured in
  place of `label`) and `AxisOptions` gains `category_groups:
  Option<CategoryGroups>` (headers over runs of categories with an
  optional bracket and a drag offset, on a categorical `top_x` or
  `bottom_x`), drawn past the tick labels and selected and dragged as
  `Element::CategoryGroupLabel` / `CategoryGroupLabelElement` (registered
  by `HitMap::standard_chart`). `Config` gains `plot_background:
  Option<Color>`, a fill under the grid. Migration: an exhaustive `match`
  on `DataRenderType` adds `EnergyLevels(_)`, one on `Element` adds
  `CategoryGroupLabel(_)`; a `PickedDataRef::ValueLabel { .. }` literal
  adds `part: None`; a `Category { .. }` literal adds `rich_label: None`;
  an `AxisOptions { .. }` literal adds `category_groups: None`; a
  `Config { .. }` literal adds `plot_background: None`.

- **Grotrian diagrams at atlas scale** ([#205](https://github.com/shim9610/figgy/issues/205)).
  A crowded categorical axis staggers its labels over rows instead of
  thinning them (as many rows as the stride would have thinned by, each one
  label extent further out), so every category keeps its name; group headers
  sit past the outermost row. `AxisOptions` gains `category_window:
  Option<[f64; 2]>`, the slots a categorical axis shows, which the view
  helpers (`set_x_range` / `set_y_range`, the subplot and overlay setters, a
  view state) set in place of `min` / `max`; `data_range` follows it, so
  ticks, grid, data, picks and annotations do, and the `auto_fit_*` family
  leaves a categorical axis as before. `AxisBreaks` gains `sizing:
  PieceSizing` (`ByValue`, the default; `Equal`; `Weights(Vec<f32>)`): with
  `Equal` or `Weights` each piece has a scale of its own (`BreakMap::scale_at`,
  `per_piece`) and walks its own tick spacing (`tick::piece_ticks`,
  `PieceTicks`); a `Panels` axis refuses any but `ByValue`, and a view whose
  end falls inside a cut leaves no zero-span piece. `EnergyLevelSeries` gains
  `bands: Vec<LevelBand>` (`id`, `column`, `first`, `last`, `label`,
  `label_offset`), each a light fill with one label over thin inner levels
  (`EnergyLevelStyle::band: LevelBandStyle { fill, inner_width }`), picked
  as its first level on the fill and as `LabelPart::Band` on the label.
  The placement is one layout per raster and pick (`LevelLayout`): level
  labels stack per lane with leader lines, a column whose labels would leave
  the plot draws them on the other side, a level outside the plot draws no
  label, a dense run of four or more labels collapses to its first and last
  joined by a bracket, a crowd (a stack that would move a label more than one
  and a half label heights) collapses its dense runs and is stacked again,
  arrows meeting one end of a level spread across its slot, and each
  wavelength label takes the first clear spot along its arrow.
  `Subplot` gains `marker: Option<InsetMarker>` (`window`, `line`,
  `connectors`): the window an inset magnifies, derived from the inset
  (`Config::inset_window`) when `None`, emitted on the parent's annotation
  layer on every draw (`Config::inset_marker_parts`, `emit_inset_markers`),
  which supersedes the static `Config::inset_marker` annotations.
  `layout::inset_parent` and `inset_paint_order` give the tree order insets
  are painted in; `Config::insets` lists them bottom to top,
  `Config::nested_subplot_at` answers the inset drawn on top, and
  `Config::subplot_at` the subplot drawn on top at a point, inset or not.
  `AxisPreset::Grotrian` is the diagrams' preset: the boxed frame with
  outward ticks and labels on the bottom and left, the grid off, and a
  continuous `left_y` break sized `1 : 9 : …` (one weight per piece, the
  lowest thin); a `Panels` break or an unbroken axis keeps its sizing.
  Subplot frames: `Config::set_subplot_area` places a subplot anywhere, and
  `Element::SubplotFrame(i)` / `SubplotFrameEdge(i, side)` move and resize
  it by whole pixels through `Config::nudge_from`, shifting the document
  when a frame passes its top or left edge. `HitMap::for_active_layer(Base)`
  now also registers every subplot's frame (`SubplotFrameElement`,
  `subplot_<i>:frame`, selected by a press on the subplot's plot) and its
  decorations over the parent's, each answering only where its subplot is
  drawn on top (new `Selectable::owns_point`, default everywhere), so a
  base-layer `hit_test` can answer a `subplot_<i>:…` id; new
  `Resizable::resize_from` carries the pointer into a resize (default
  `resize_by`). Migration: an exhaustive `match` on `Element` adds
  `SubplotFrame(_)` and `SubplotFrameEdge(..)`, one on `LabelPart` adds
  `Band` and one on `AxisPreset` adds `Grotrian`; an `AxisOptions { .. }`
  literal adds `category_window: None`, an `AxisBreaks { .. }` literal
  `sizing: PieceSizing::ByValue`, an
  `EnergyLevelSeries { .. }` literal `bands: Vec::new()`, an
  `EnergyLevelStyle { .. }` literal `band: LevelBandStyle::default()` and a
  `Subplot { .. }` literal `marker: None`; a host that keys on the base
  layer's hit ids sees the new `subplot_<i>:` ids only on a document with
  subplots. Documents written before these fields parse and re-serialize
  unchanged. The chrome emit takes the display or export scale the config's
  pixels carry: `chrome::emit_chart_chrome(config, measure, sink, scale)`,
  `chrome::emit_part_of(config, element, measure, sink, scale)`,
  `chrome::line::emit_axis` / `emit_axis_strokes(.., sink, scale)` and
  `each_axis_stroke(.., majors, scale, each)`. Only a broken axis reads
  it: with `frame_each_panel` each piece frame is snapped to whole logical
  pixels and then scaled, so it is one crisp row at 1x and 2x is 1x
  doubled; the hit map passes 1, within a logical pixel of the drawn frame
  and inside the stroke grab width. Migration: a caller of any of these
  passes `1.0` for a config drawn as stored, or the scale it scaled the
  config by.

- **Triangle mesh series** ([#210](https://github.com/shim9610/figgy/issues/210)).
  `DataRenderType::TriMesh(TriMeshSeries)` (new module `tri_mesh`) is a
  field over host-given triangles: `triangles` names a triangle index
  column, `values: MeshValues { column, at }` the value column and whether
  its values sit at the `MeshValueLocation::Vertices` or `Faces`, and
  `fill`, `contour` and `surface` are the heatmap's `FieldFillConfig`,
  `ContourConfig` and `SurfaceStyle`; `x_column` / `y_column` are the
  vertex coordinates and `z_column` the vertex height on a 3D chart.
  `TriMeshSeries::validate` and `Config::validate_tri_mesh_series` (in
  `validate_series`) hold the rules: vertex values shaded `Interpolated`,
  face values `Flat`, contour lines only over vertex values and without
  labels, no surface `color_matrix`, not on polar, ternary or parallel
  charts, and on a 3D chart a `z_column` and a continuous fill. A picked
  triangle is `PickedDataRef::MeshTriangle { source_id, series_id,
  triangle_index }` (serde tag `mesh_triangle`); `step_selection` moves it
  by `dx` over the triangles (the triangle index column's `column_len` over
  three) and by `dy` across the mesh series. The legend mark and the colour
  cycle treat a mesh as a field. `Config::drawn_z_column` names a mesh's
  `z_column` on a 3D panel, as it names a scatter, line or point vector
  series' (the trisurf, #210 step 5), so `step_selection` and the renderer
  bound the draw by the height column too. Migration: an exhaustive `match`
  on `DataRenderType` adds `TriMesh(_)`, one on `PickedDataRef` adds
  `MeshTriangle { .. }`.

<a id="unreleased-renderer"></a>

### Renderer (`figgy-renderer` 0.19.0)

- **`prepare` redraws a stale raster itself; the chart holds its selection** ([#265](https://github.com/shim9610/figgy/issues/265)). Breaking.
  - `ChartDrawItem` is now `{ view: &mut ChartView, chart: &Chart, series }` (was `view: &ChartView, chart_config: &Config`), and `RegisteredChartDrawItem.view` is `&mut ChartView`. `Renderer::prepare`, `prepare_registered`, `paint`, and `WindowedRenderer::draw`, `draw_registered`, `first_frame_ready(_blocking)` and `warm_up(_blocking)` take `&mut [..]`.
  - `prepare` redraws a view's text and axis raster first when it is stale: its chart's raster stamp, panel rect, scale or the font generation differ from the ones it was drawn with. An unchanged view is left alone. The stamp is an issued number every raster-affecting `Chart` edit moves, so two charts never share one.
  - `Chart::select(Option<&str>)` / `Chart::selected()` hold the element the raster draws selected, by element id. `prepare_registered` draws a registered chart's stored selection (`set_chart_selection`) at its display placement.
  - `refresh_axis*` stay for a host that draws the raster itself; a refresh of the chart as `prepare` would draw it is not repeated.
  - A failed `prepare_registered` (an unknown chart) redraws no view.
  - A view made with `create_chart_view` and drawn by `prepare_registered` is not redrawn on its first frame when the stored chart matches: a redraw whose panel and annotation raster keys all match uploads nothing, and a registered chart's annotation edit redraws the annotation layer alone. A view with fewer overlay layers than the stored chart grows on the next prepare instead of failing with `StaleStateToken`.
  - **Migration:** build the item with `view: &mut view, chart: &chart` (wrap a bare `Config` in `Chart::new`), pass `&mut [..]`, set the selection with `chart.select(..)` instead of `refresh_axis_with_selection`, and drop the host's dirty-flag `refresh_axis` branch.

- **Chart placement, overlays, swatches, a headless radial renderer and a PNG save** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature: `Chart::placed(x, y, width, height)` and `with_area(..)` (resize a chart built from a document), `set_area(..)` (move an existing chart's area in place, as on a window resize), `with_overlay(y_axis, &series_ids)`, `with_legend_swatch(label, color)` for a legend entry that is not a series, `RadialRenderer::headless` / `headless_blocking`, and `RasterImage::save_png(path)` for a host that checks pixels before writing them.

- **The examples use the helpers** ([#265](https://github.com/shim9610/figgy/issues/265)). The headless examples make their renderer with `Renderer::headless_blocking`, pass plain `Vec`s to `add_column`, build series with the `SeriesConfig` constructors and modifiers, take legends from their series (`with_legend_from`) and save with `save_blocking` where they need no image check. The gallery look and the designed scenes' chrome are themes (`examples/support/gallery.rs`, `support/styled.rs`). The examples shrink from 15,116 to 13,683 lines; the quick start (`examples/line.rs` and the README) from 85 to 30. Gallery images change only where a legend symbol now shows its series' own mark, where an axis title takes the gallery theme's ink, and where the dark style previews take the dark theme's chrome. `winit_simple` and `constellation_lab` leave the raster redraw to `prepare` and set the selection on the chart.

- **Chart themes** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature: `renderer::theme` re-exports the model's themes, `Chart::apply_theme(&theme, &mut series, SubplotScope)` applies one to the chart and its series and refits the layout to the new text sizes, and `Chart::with_theme(&theme)` applies the chrome alone and fits.

- **Math in the text wrappers** ([#272](https://github.com/shim9610/figgy/issues/272)). Breaking. `Chart::with_title`, `with_x_title`, `with_y_title`, `with_legend_entry`, and with the `helpers` feature `with_colorbar_title` and `with_legend_swatch`, read their text through `text::rich_segments_from_label`: `with_y_title(r"$\sin(2\pi f t)$")` draws a formula, and a title's band and `fit_layout` measure it as drawn. `with_legend_from` carries each series' label as it is, math included. The examples that built TeX segments by hand (`line`, `error_bars`, `area_step`, `vector_field`) use the wrappers, and `examples/support/gallery.rs` drops its `tex_label`. **Migration:** a `$` in a string passed to these wrappers (and to `SeriesConfig::label`, `Annotation::text`, `AxisOptions::overlay_y`, `RadialChart::with_title` and the browser text setters) now opens math; write `\$` (in Rust, `r"Price (\$)"` or `"Price (\\$)"`) for a dollar sign. A lone `$` with no closing one still shows as written, since its TeX does not parse. Text set on the document directly (`RichText::plain`, a JSON `RichText`) is unchanged.

- **Axis units** ([#273](https://github.com/shim9610/figgy/issues/273)). The renderer draws, hits, selects, lays out and exports the tick labels of an axis with a `unit` in its shown units (see the model entry); the data, the column pool and the picks are untouched. `Chart::set_x_range`, `set_y_range`, the `auto_fit_*` family and a view state choose `major_spacing` for the shown range of an axis with a unit. A contour label still prints its level in data units. A unit edit is an axis chrome edit: the axis raster is redone and the data is not touched. The renderer formats no cursor or hover readout of an axis value; the playground's hover readout prints the shown values, and its Chart section has a scale and offset per axis pair. With the `helpers` feature, `Chart::with_x_unit(scale, offset)` and `with_y_unit(scale, offset)` set the unit of `bottom_x` / `left_y` (the paired `top_x` / `right_y` keep theirs, as with `with_label_format`) and choose the spacing for the shown range as the range setters do; a unit the axis refuses is stored for `Config::validate_chart` to name, with no panic. The test catalogue gains the `AxisUnits` scene (counts printed as volts, samples as milliseconds, an overlay's kelvin as °C), which the decoration oracle, the auto layout sweep and the vector export sweep cover.

- **Tick label clearance** ([#272](https://github.com/shim9610/figgy/issues/272)). The renderer draws, hits, selects and lays out the tick labels the model keeps at axis corners and break marks (see the model entry). With the `helpers` feature, `Chart::with_label_clearance(LabelClearanceMode)` sets `label_style.clearance` on every axis (`bottom_x`, `top_x`, `left_y`, `right_y`, which is a 3D chart's z axis). A clearance edit is a label style edit: the axis raster is redone and the data is not touched. The playground's Chart section has "keep X labels" and "keep Y labels".

- **Radar value labels stay next to their vertices** ([#272](https://github.com/shim9610/figgy/issues/272)). A radar value label sits outside its vertex along the spoke, `max(4, 0.25 em)` past the marker (was 9 px from the vertex), and still flips inward at the outer ring or the data area's edge. Only labels that would touch move, and only next to their own vertex: of two labelled series on one spoke, the larger value's label stays outside and the smaller value's goes inside its own vertex; equal or nearly equal values that would still touch split to the two sides of the spoke at the vertices' radius; a label that would touch the ring labels goes inside or beside its vertex. More series on a spoke are placed the same way from the outermost vertex inward. The shader reads the other labelled series' values at the spoke on every draw (SHADER_COMMON.md §5.17); nothing is stored, and there is no cap on the series. Drawing, picking and the selection box share the placement.

- **Chart conveniences** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature: `Chart::sized(width, height)` (the documented default look with that chart area), `with_legend_from(&series)` (one entry per labelled series, its symbol from the series through `legend::series_symbol_segments`, so a colour is written once) and `fit_layout()` (`layout::auto_layout` with the chart's own font measure).

- **A headless renderer in one call and one save call by format** ([#265](https://github.com/shim9610/figgy/issues/265)). With the `helpers` feature: `Renderer::headless(HeadlessOptions)` (async, browser too), `Renderer::headless_blocking()` and `headless_blocking_with(options)` (native) make the instance, adapter and device (honouring `WGPU_BACKEND`) and an `Rgba8Unorm` renderer. `ExportFormat { Png, Svg, Pdf }` with `from_path` and `extension`; `Renderer::export_bytes` / `export_bytes_blocking`, `save_blocking` (format from the extension) and `save_as_blocking`, and the same on `RadialRenderer`. They route to the existing exports. A save fails with `SaveError` (`Export`, `Radial`, `UnknownFormat`, `Write`). `renderer::helpers` is now the renderer's own module: it re-exports the model's helpers and holds these, and the host `Default` for `ExportOptions` and `ExportScope` moved there.

- **`Vec`s and slices upload as they are** ([#265](https://github.com/shim9610/figgy/issues/265)). `ColumnSource` is implemented for `[T]`, `&[T]` and `Vec<T>` with `T` = `f64`, `f32`, `u32` and `Option<f64>`, and `HiLoColumnSource` for the `f64`, `f32` and `Option<f64>` ones, so `add_column("x", &xs)` takes the host's vector without a `Column` wrapper. Values go straight from host memory into mapped staging. `Column<T>` now uploads through the same slice implementations. Arrays are not implementors: a trait `len` on `[T; N]` would shadow the slice's.

- **No default fallbacks in the renderer; `helpers` feature** ([#266](https://github.com/shim9610/figgy/issues/266)).
  - The renderer depends on the model without `helpers` and forwards its own `helpers` feature, which is on by default.
  - Category bar and box plot layout reads `Config::effective_bar_groups()`. Before, the same fallback was written six times, each cloning the document's `bar_groups` and inventing a default.
  - The 3D surface look reads `SurfaceStyle::resolved`.
  - The radial label raster key comes from the model's `label_raster_key` functions.
  - "No annotations" is `AnnotationLayer::empty()`.
  - `ExportOptions::panel` and `document` no longer go through `Default`. `impl Default` for `ExportOptions` (`document(1.0)`) and `ExportScope` (`Document`) now sit behind `helpers`.
  - `renderer::default` and the new `renderer::helpers` re-export the model's `helpers` folder and exist only with the feature.
  - **Migration:** a build with `default-features = false` adds `features = ["helpers"]` to keep `Default` on the model types, `renderer::default` and `ExportOptions::default()`.

- **Wavelength labels and corner notes** ([#258](https://github.com/shim9610/figgy/issues/258)).
  The energy-level raster and the SVG and PDF export draw the labels
  `wavelength_labels` writes, through the same placement as host labels.
  The annotation layer places a text block by its `align`. `NoteAlign` and
  `WavelengthLabels` are re-exported. The NIST examples and the
  energy-level scenes label their lines from `wavelength` to one decimal
  and show the unit once, in a corner note.

- **Gallery findings** ([#259](https://github.com/shim9610/figgy/issues/259)).
  - A 3D floor contour (a heatmap's or a mesh's) is `line_width` wide on
    screen whichever way it runs. It was measured in floor-plane pixels, so
    the strokes running across the view thinned out and broke up below
    about 2 px.
  - A colourbar title that the band's centre would lay over the tick labels
    sits a third of the label font past them instead, as far as its band
    reaches; `auto_layout` places it from the same base.
  - A hexagonal grid takes the nearest centre in the lattice's own metric,
    so a lattice stretched along one axis draws stretched hexagons rather
    than slabs. A lattice regular in data units draws the same cells.
  - The default `AxisBreaks::gap_px` is 8 px.
  - `PolarOptions.hide_first_radius_label` (default `false`, omitted from
    JSON) skips the innermost radius ring's label, as on a wind rose's calm
    circle.

- **Typed validation reasons** ([#215](https://github.com/shim9610/figgy/issues/215)).
  `FiggyError::InvalidConfig { field, kind }` and
  `FiggyError::InvalidSeriesConfig { series_id, kind }` carry the model's
  `RuleKind` (re-exported as `renderer::RuleKind`) in place of
  `reason: &'static str` and `reason: String`; the renderer's own rules
  (registered columns and their lengths, records, ragged and triangle
  columns, streamed sources, GPU index spaces, colourbars, line fills, the
  radial chart) are variants of the same enum, so a host matches one type.
  `RadialError::Invalid` carries a `RuleKind`. Every message reads as
  before, byte for byte; a test pins each unit variant's sentence. Migration:
  replace `InvalidSeriesConfig { reason, .. }` with
  `InvalidSeriesConfig { kind, .. }` and compare `kind` with a variant (or
  call `kind.to_string()` for the text); code that built these errors builds
  them from a `RuleKind`.

- **Feynman diagrams are picked and dragged** ([#234](https://github.com/shim9610/figgy/issues/234)).
  A typed data pick answers for a Feynman series on the CPU over the drawn
  placement: `PickedData::FeynmanVertex`, `FeynmanPropagator` and
  `FeynmanLabel` (and `to_ref` into the `PickedDataRef` variants). The
  picked part is drawn in the series' picked style.
  `Renderer::drag_feynman(chart, series_id, FeynmanHit, dx, dy)` moves a
  vertex (the first drag commits the layout) or bends a propagator in the
  series' panel space; a label moves in logical pixels through
  `FeynmanSeries::drag_label`, the value-label drag, and the panel drag
  refuses it. Migration: an exhaustive match on `PickedData` adds the three
  arms.
- **Feynman diagrams are drawn** ([#234](https://github.com/shim9610/figgy/issues/234)).
  A `DataRenderType::Feynman` series is drawn on the CPU through the
  model's emit into the DataLess raster of its panel, as an energy-level
  series is, at the layout's positions until they are committed, and is
  written as vector parts in SVG and PDF. `Renderer::auto_layout(chart,
  series_id)` commits the layout into the registered series, which the host
  reads back through `chart_series`. `auto_fit_all` fits the diagram's
  vertices. The auto-fit token no longer reads the empty columns of a
  DataLess series, which also lets `auto_fit_all` run on a chart with an
  energy-level series. A 5,000-rung ladder (10,004 vertices, 15,002
  propagators) lays out in about 0.5 s and draws at 1x in about 0.8 s in
  the debug test profile; nothing is capped.
- **DataLess series are vector content in SVG and PDF** ([#234](https://github.com/shim9610/figgy/issues/234)).
  An energy-level series is written as vector parts from the emit its
  raster uses (levels, limits, transitions, arrowheads, labels and the
  picked row), clipped to each data piece of its panel, instead of inside
  the data image. It sits where the frame draws it, between the panel's GPU
  data and its value labels: a panel group with a DataLess series is
  written as one image per run of GPU steps between two panels' DataLess
  series, with the series between them, and a run with no GPU series
  writes no image. `paint_export` takes the steps of each panel's data slot
  it paints (`DataSteps`). The PNG export is unchanged. Migration: the SVG
  and PDF of a document with an energy-level series hold vector parts where
  they held its pixels; a reader of the file that expected one data image
  per panel group finds one per run.
- **Triangle index columns** ([#210](https://github.com/shim9610/figgy/issues/210)).
  A fourth pool source kind, for the coming triangle mesh series:
  `Renderer::upsert_triangles(id, &TriangleSource { indices })` and
  `remove_triangles(id)` (also on `ColumnPool`, with `triangles(id)` and
  `triangle_stats(id)`). Three vertex indices per triangle upload as one
  member column of `u32` index lanes (SHADER_COMMON.md §5.14), exact up to
  `u32::MAX`. The upload's write observer rejects an empty column, a length
  that is not a multiple of 3, an index that is not an integer in
  `0..=u32::MAX` and indices written out of order, before anything is
  published, and records `TriangleStats { triangles, min_index, max_index }`;
  no index is kept on the CPU. The id shares the one id space of columns,
  records and ragged columns, the member is internal, and the kind is
  resident only: a streamed id is refused both ways.
  `Column<u32>` implements `ColumnSource`, writing each value's exact
  `(hi, lo)` split. The streaming refusal for composite ids now reads
  "streaming is not extended to records, ragged and triangle columns".

- **Triangle mesh series: the flat draw** ([#210](https://github.com/shim9610/figgy/issues/210)).
  A `TriMesh` series draws its fill and its contours on a flat chart: one
  instance per triangle through a new assembled module (`tri_mesh`, entries
  `vs_tri`, `fs_tri`, `fs_tri_contour`) that reads the field's group 2
  layout under `FIELD_FLAG_MESH` / `FIELD_FLAG_FACE_VALUES`
  (SHADER_COMMON.md §5.25). The fill colours through the heatmap's new
  `field_value_color`, the contours through its level search
  (`contour_block_candidates`, `contour_level_over`, moved into
  `field_paint.wgsl`; the heatmap's pixels are unchanged). Registration
  checks the triangle column, the value column, the colourbar, the contour
  level count and that every index names a vertex; the fit covers the
  vertex columns; `series_draw_info` counts the triangles and reports a
  column shortened after registration. A streamed display refuses the kind
  ("triangle mesh series require GPU-resident columns; streamed sources are
  not supported", rule 8). The model's `TriMesh` series kind and `MeshTriangle` selection are
  re-exported (`renderer::tri_mesh`); `PickedData::MeshTriangle { source_id,
  series_id, triangle_index, vertex_index, distance_px }` is the pick result
  to come, `to_ref` dropping the hover-only `vertex_index`. Migration: an
  exhaustive `match` on `PickedData` adds `MeshTriangle { .. }`.

- **Triangle mesh series: picking and the picked highlight** ([#210](https://github.com/shim9610/figgy/issues/210)).
  The typed data pick answers a mesh's triangles through a new compute entry
  of the mesh module (`pick_tri_mesh`, one 64-lane workgroup over every
  triangle, SHADER_COMMON.md §5.25): `PickedData::MeshTriangle` with the
  triangle and its vertex nearest the cursor, at distance 0 inside the
  triangle and within one output pixel of its edge, only where the fill
  paints, the later triangle on overlap. A contour stroke within the pick
  distance answers first as `PickedData::ContourLevel` with the triangle in
  `x_index`. The picked triangle is outlined inside its edges at
  `outline_width_px`, and a picked level redrawn `contour_width_extra_px`
  wider, through the mesh's selection entry (`fs_tri_selection`);
  `ColumnFieldSelectionLayer` gains `vertices` and `instances` for it. A
  million triangles pick in about 50 ms on lavapipe; nothing caps the count.
  Migration: code that builds a `ColumnFieldSelectionLayer` sets `vertices:
  FIELD_VERTICES, instances: 0..1` for a field.

- **Triangle mesh series: the 3D surface (trisurf)** ([#210](https://github.com/shim9610/figgy/issues/210)).
  On a 3D chart (`Config.view_3d`) a `TriMesh` series draws as a surface
  over its triangles, each vertex placed by `x_column`, `y_column` and the
  series' `z_column` through the camera, coloured by its values as the flat
  fill is, with `TriMeshSeries.surface` as the look and the triangle's own
  normal for a lit material; its contours draw on the box floor. The
  surface entries (`surface_mesh/surface_entry.wgsl`) now draw what an input
  fragment defines, the heatmap grid's (`surface_input_grid.wgsl`,
  `SURFACE_SHADER`) or the mesh's (`surface_input_mesh.wgsl` over
  `tri_mesh/mesh_input.wgsl`, the new `TRI_SURFACE_SHADER`; `SurfaceInput`
  names which), through the hooks `surface_edge_px`, `surface_pick_ids`,
  `surface_selected_vertex` and `surface_patch`; `SurfaceParams` moved to
  `surface_params.wgsl` (SHADER_COMMON.md §5.21). The grid surface's pixels
  are unchanged. The mesh module gains the floor entries `vs_tri_floor`,
  `fs_tri_floor` and `fs_tri_floor_pick` (`tri_mesh/mesh_floor.wgsl`) and
  splices `project_3d.wgsl`; the mesh's group 2 gains the height column as
  its third table entry on a 3D chart (`MESH_TABLE_HEIGHTS`), so a new
  height upload redraws the surface. The ID pick writes `(id, 2, triangle,
  vertex)` for a mesh surface and `(id | 0x80000000, level, triangle, 0)`
  for a mesh floor contour, decoded to `PickedData::MeshTriangle` and
  `ContourLevel { x_index: triangle, y_index: 0 }`; the picked triangle is
  the surface's patch, outlined and brightened over anything in front of
  it. Registration checks the height column and counts it in the vertex
  count, `series_draw_info` too, and `auto_fit_view_3d_z` spans it (it
  already read every series' `z_column`). `ColumnSurfaceLayer` carries
  `vertices` and `instances` (a grid's `SURFACE_QUAD_VERTICES` and quads, a
  mesh's `TRI_MESH_VERTICES` and triangles); `SurfacePass::label` and
  `create_surface_pipeline` take the `SurfaceInput`; `TargetPipelines`
  gains `tri_surface`. Nothing caps the triangle count. Migration: code
  that built a `ColumnSurfaceLayer` sets `vertices: SURFACE_QUAD_VERTICES`
  and renames `quads` to `instances`.

- **Triangle mesh scenes and guide** ([#210](https://github.com/shim9610/figgy/issues/210)).
  The playground scenes `SceneKind::TriMesh` (potential flow past a
  cylinder on a graded annulus mesh, a mesh with a hole: the host's pressure
  coefficient per node, interpolated across the triangles, with nice
  contour levels) and `SceneKind::TriSurface` (the same mesh as a matte,
  wireframed surface in a 3D box, its contours on the floor) join
  `SceneKind::ALL`, so the decoration oracle, the automatic layout sweep,
  the vector export sweep and the playground's self-test cover them. The
  new [triangle mesh guide](crates/renderer/TRIMESH.md) collects the input,
  the series, the drawing, the 3D surface, the interactions and the tests;
  the interaction and API guides, the READMEs and the model README name the
  kind.

- **A frame of more series than a scratch cache holds draws them all.**
  The per-series scratch caches (256 entries each) cleared themselves when
  the 257th series of one frame was prepared, so that frame's layers lost
  the earlier series' scratches and the frame failed with an internal
  error: more than 256 heatmaps, images, vector series, colour-mapped
  scatter, streamline or parallel series (or triangle meshes) in one frame
  did not draw (rule 10). Each cache (`field`, `vector`, `value_map`,
  `contour`, which also holds value label atlases and energy-level
  rasters) is now bounded before a frame resolves, keeping the series that
  frame draws, and evicts nothing while it runs; a lone preparation (a
  pick) still evicts as before. The contour cache never failed (its frame
  keeps snapshots), and now also stops rebuilding the frame's own entries.
  Tests draw 300 heatmaps, vector series, colour-mapped scatter series,
  contours and triangle meshes in one frame.

- **Shaders compile on the renderer's own thread** ([#222](https://github.com/shim9610/figgy/issues/222)).
  A backend's shader compiler can need more stack than the calling thread
  has: on Windows WARP the largest pipelines need 2 to 4 MiB, and a Windows
  main thread has 1 MiB, so pipeline creation could overflow the host's
  stack. Natively the renderer now creates every shader module and pipeline
  on a scoped thread with a 16 MiB stack (reserved, not committed). Errors
  still reach the caller's error scope: the compile thread captures them and
  the creation is repeated on the calling thread. No API change; the browser
  build is unchanged.

- **A field picks in two compute dispatches** ([#222](https://github.com/shim9610/figgy/issues/222)).
  On Windows WARP, picking a field overflowed the stack of a thread that WARP
  creates with the 1 MiB default, past the compile thread above. The single
  entry point held both the contour and the cell pick; each half alone fits.
  The entry is split into
  `pick_field_contour` and `pick_field_cell`. One pass dispatches both, in
  that order, and the cell keeps the contour's candidate when there is one,
  so the result is unchanged. A host that observes initialization
  (`InitEvent::stage`) now sees these two stages in place of
  `pick_field_data`.

- **Probability axes are refused until drawn** ([#209](https://github.com/shim9610/figgy/issues/209)).
  A chart with a probability axis on any axis or subplot is refused at
  registration and at export with `InvalidConfig` ("probability axes are not
  drawn yet"), rather than drawn as a linear axis, until the GPU transform
  places them. The annotation space, the fit stamp (codes 2 to 4) and the
  render warnings take the new scale.

- **The partner axis takes the main axis's scale.** `Chart::set_x_range`,
  `set_y_range`, the `auto_fit_*` helpers (and the renderer's fit applied
  to a registered chart) and the subplot range setters write the range to
  both axes of a pair. They kept each axis's `scale` and chose both axes'
  major spacing, minor count, label format and precision from the main
  axis's (`bottom_x`, `left_y`). Beside a logarithmic `left_y`, a linear
  `right_y` over 1..1000 therefore got a one-unit spacing with eight
  minors, about 8000 ticks drawn as a solid bar down the right edge, and a
  logarithmic partner beside a linear main axis drew no decade inside the
  range. The helpers now give `top_x` and `right_y` the main axis's scale
  first, then each axis the ticks of its scale, so a logarithmic `left_y`
  puts decades on both edges. A categorical partner keeps its scale, and a
  3D or ternary chart's `top_x` and `right_y` (the z and c axes) keep their
  own scales and get the ticks of them. View states apply each axis's scale
  as they carry it. Migration: a chart that wants a second value axis with
  a different scale on the right uses an overlay layer (`Config::overlays`);
  a `right_y` or `top_x` scale that differs from its main axis's is
  replaced by the next range helper call.

- **Probability axes are drawn** ([#209](https://github.com/shim9610/figgy/issues/209)).
  `ScatterTransform` gains `scale_prob: [f32; 4]` at offset 288 (288 B →
  304 B): the probability code of x and y (0 none, 1 normal, 2 Weibull,
  3 logistic). On a probability axis `scale_log` is 0 and `data_min` /
  `data_max` hold `F⁻¹` of the bounds. The new fragment
  `common/probability.wgsl` (spliced wherever `projection.wgsl` is) places a
  sample at `F⁻¹(p)` from its `(hi, lo)` pair, so the upper tail's
  `1 − p` survives: AS241 `PPND7`, the Weibull series below 0.01, the logit,
  their slopes for the vector and line-head derivatives and their inverses
  for the contour label anchors. `axis_pair_to_t` and
  `axis_px_per_unit_at` take the code after `is_log`; `sample_valid` and the
  pick omit a value at or past 0 or 1; an area or stem baseline there is
  clamped to the axis end on its side. The range helpers
  (`apply_axis_range`) write `LabelFormat::Percent` and `minor_count = 1` on
  a probability axis; an auto fit keeps its ends in `(0, 1)` (an end at 0 or
  1 takes the mirror of the other) and pads in `F⁻¹` units; line offsets
  shift fit extents in `F⁻¹` units; a view state checks the bounds. A
  streamed series on a probability axis is refused ("probability axes
  require GPU-resident columns; streamed sources are not supported"). The
  model PR's "not drawn yet" refusal is gone. The annotation space, the fit
  stamp (codes 2 to 4) and the render warnings take the new scale.
  Migration: a `ScatterTransform` literal adds
  `scale_prob: renderer::data_render::NO_PROBABILITY` (`[0.0; 4]`).
  The `probability_paper` example (`gallery-probability.png`) draws normal
  paper and Weibull paper over a log time axis; the playground has the same
  scene and an axis scale menu in place of its log checkboxes.

- **Stems** ([#206](https://github.com/shim9610/figgy/issues/206)). A
  series' stems are the error-bar module's second draw: one quad per point
  (`vs_stem`, `vs_stem_sketch` into `fs_stem`; a Milkyway jet through
  `vs_jet`'s stem branch), built by `stem_segment` in
  `common/errorbar_segment.wgsl` from the baseline to the point. Its
  `Style` carries `ERRORBAR_STEM` (bit 5 of `primitive_flags`) with the
  direction bit, and the baseline as the pool's `(hi, lo)` pair in
  `cap_half_px` / `cap_width_px`. Stems draw first of the series' parts,
  under the error bars, the line and the markers, exactly where the
  marker's sample is valid. Every stem's dash starts at its baseline, with
  no arc scan. On a logarithmic value axis a baseline at or below zero is
  drawn at the axis minimum: `axis_baseline_ndc` in `common/projection.wgsl`
  now serves the area fill and the stems. A stem picks as its point
  (`pick_stem_candidate` under the error-bar pick variant; a dash gap does
  not pick), and the marker wins a tie. The baseline joins the value
  axis' auto-fit, as a histogram's does (`FitExtentParams::Stem`), before
  a line offset member's shift. Constellation drops a stem series, as it
  drops an error-bar series. Streaming is not extended to stems
  (`STREAMED_STEM_REASON`). Every row draws a stem: 1,000,001 rows draw in
  about 1.2 s and pick their last row in about 4.7 s on lavapipe, measured
  by `tests/stem_render.rs` and never capped.

- **Polar heatmaps and contours** ([#208](https://github.com/shim9610/figgy/issues/208)).
  A field on a polar chart draws each cell as the annular sector between
  its angle and radius boundaries. The field draw stays one quad: each
  fragment's pixel offset from the centre is inverted by `polar_unproject`
  (`common/projection.wgsl`) and read by `field_lookup`
  (`common/field_grid.wgsl`), which wraps the angle into the turn from the
  grid's low boundary and carries the lookup's per-pixel Jacobian. The
  fill, the cell pick, the picked cell's outline (the rose's
  `radial_sector.wgsl` footprint, now spliced into the field module), the
  contour stroke (the Euclidean pixel distance to the isoline, by
  foot-point iteration on the field through the polar placement), the
  contour pick and highlight, and the contour label
  anchor pass (seeded and stepped in (angle, radius); the anchor module
  now splices `sample_validity.wgsl`) all walk this one lookup. A
  rectangular chart's lookup is the identity and its path is unchanged.
  Edges and centres, both orientations, flat and interpolated shading, log
  and inverted radius axes, a categorical angle axis, the hole and the
  sector work; an interpolated full circle closes its seam when the host
  names the first column again one turn on. Streaming stays refused for
  every polar chart. No count limit: 65,537 angle cells by 64 rings draw
  in about 0.4 s and a 2,048 by 2,048 grid in about 0.5 s on lavapipe,
  measured by `tests/polar_heatmap_render.rs`. The `PolarHeatmap` scene is
  a weather radar sweep and `polar_plot` writes a fifth image, an antenna
  gain heatmap. The shader prewarm contract is regenerated.

- **3D vector fields** ([#211](https://github.com/shim9610/figgy/issues/211)).
  A point vector on a 3D chart draws one arrow per row from `(x, y, z)`
  along `(u, v, w)`, depth tested against the surfaces, the 3D marks and
  the other arrows. The new assembled module `VECTOR_POINTS_3D_SHADER`
  (`VectorInput::Points3d`) is the point form with the flat projection
  fragment replaced by `common/project_3d.wgsl` and the new
  `vector/project_box3d.wgsl`, plus the ID pick entries of the new
  `vector/vector_entry_3d.wgsl`; the glyph and the input are unchanged
  (rule 9). Both projection fragments now define `arrow_clip(corner,
  arrow)`, and `ArrowPx` carries the NDC depth at the tail and tip; the
  entry's body is `vector_vertex`, and the 2D vector modules draw
  byte-identical pixels. The glyph stays camera-facing in logical pixels.
  `Data` tips are placed through the axes and the camera, and an arrow
  whose anchor or tip leaves the box is not drawn; `Pixels` length is the
  screen-plane length at the anchor. `VectorParamsGpu` grows from 128 to
  144 bytes (`height_base`, `height_len`, `w_base`, `w_len`) and
  `VECTOR_FLAG_THREE_D` (64) selects the height and `w` reads.
  `View3dPipeline` gains `Vector` and `VectorPick` (`VIEW3D_PIPELINES` 10).
  The 3D ID pick answers an arrow's row where its shaft or head covers at
  least half the pixel, the picked ring sits at the anchor at half the head
  width, and the z fit adds `[z_min + f·w_min, z_max + f·w_max]` for a
  `Data` arrow from column statistics. `SeriesDrawInfo` counts the height
  and `w`, and registration checks `w` on a 3D chart. A change of a
  series' `z_column` is now a data declaration change (it moved no
  revision before). Streamed sources are refused. No count limit:
  `(1 << 20) + 1` arrows draw, pick their last row and step to it in
  `tests/surface_render.rs`. The `Quiver3d` scene draws an
  Arnold-Beltrami-Childress flow coloured by a host-computed speed. The
  shader prewarm contract is regenerated.

- **Flat radial charts shade most pixels once** ([#41](https://github.com/shim9610/figgy/issues/41)).
  A radial chart whose sectors are all Flat, level (no tilt or depth),
  unlit and unoutlined shaded every pixel once per antialiasing sample (4).
  A pixel whose center lies at least 0.75 px inside or outside every
  sector's footprint now takes one evaluation, which equals each sample's;
  pixels near an edge, and every other style, still take all the samples.
  The image is byte-identical. The #41 benchmark's donut on lavapipe (Mesa
  25.2.8, 4 CPUs), warm pass, median of 15 frames in each of three runs:
  480×300 with 3 slices fell from 14.1–19.4 to 4.1–4.6 ms, 960×600 with 3
  slices from 60.5–63.5 to 14.7–19.3 ms, and 480×300 with 12 slices from
  38.5–46.4 to 11.6–14.0 ms. A chart whose first sector is not Flat skips
  the check: on Apple M4 (Metal) the check alone cost a Ceramic donut 5–8%
  per pass. No API changes.
- **A stream retry waits for its staging slot** ([#154](https://github.com/shim9610/figgy/issues/154)).
  When a chunk's `ColumnSource` fails its write into a reused staging slot,
  the slot remaps before it can take data again. A retry of the same ticket
  before that remap completed failed with
  `StaleStateToken("…WrongState")`; it now returns
  `StreamingProgress::Backpressure`, the answer a fresh request gets when no
  staging slot is free, and succeeds once the remap has run. No API changes.
- **Native GPU waits: awaited, blocking or checked** ([#160](https://github.com/shim9610/figgy/issues/160)).
  On native an awaited readback no longer polls the device on the awaiting
  thread: `RendererDevice::new` starts one poll thread per device bundle,
  which completes every awaited wait, so the future returns `Pending` until
  the work is done. The thread holds no lock (two atomic counters and its
  park token), parks while nothing is pending and exits once the bundle's
  clones, its renderers and engines and every pending ticket are gone;
  `tools/loom-poller` model-checks its protocol with loom. Each wait now
  comes in three forms with the same result, sharing the work's submit and
  its synchronous finish:
  - awaited: `resolve`, `export_rgba` / `export_png` / `export_svg` /
    `export_pdf`, `wait_submitted_work`, as before;
  - blocking (native): `GpuPickTicket::resolve_blocking`,
    `GpuDataPickTicket::resolve_blocking`,
    `GpuSeriesExtentTicket::resolve_blocking`,
    `GpuErrorbarExtentTicket::resolve_blocking`,
    `RadialPickTicket::resolve_blocking`,
    `Renderer::export_rgba_blocking` / `export_png_blocking` /
    `export_svg_blocking` / `export_pdf_blocking`, the same four on
    `RadialRenderer`, and `Renderer::wait_submitted_work_blocking`. They poll
    the device on the calling thread; no executor is needed;
  - checked: `try_resolve(self)` on each of the five tickets returns the new
    `TryResolve::Ready(result)` or `TryResolve::Pending(ticket)` without
    waiting, for a UI that checks a hover pick each frame.

  A streamed series' pick resolves by `resolve` only: its blocking and
  checked forms fail with the new `GpuPickError::Streamed` (rule 8).

  Every other call that waited for the GPU or initialized pipelines
  asynchronously gains a native `_blocking` twin that shares its steps:
  `AutoFitAllTicket::resolve_blocking` and `try_resolve`,
  `Renderer::release_unused_pool_memory_blocking`,
  `WindowedRenderer::first_frame_ready_blocking` / `warm_up_blocking`,
  `Renderer::prewarm_all_blocking` / `prewarm_all_observed_blocking`,
  `PipelinePreparation::compile_blocking`,
  `Renderer::ensure_errorbar_extent_engine_blocking`,
  `Renderer::for_window_blocking` / `for_window_blocking_observed` (and
  `WindowedRenderer::for_window_blocking_observed`), and
  `gpu::request_adapter_blocking`, `gpu::request_adapter_for_surface_blocking`,
  `gpu::request_device_blocking` and `gpu::request_device_with_blocking`
  (a host's own `DeviceDescriptor`). The asynchronous initializers keep
  their browser waits; on native nothing in them waits, and the twins run
  the same steps without an executor. The repository no longer depends on
  `pollster`: examples, the playground and the guides use the blocking
  forms, and the tests run the async API on a small executor in
  `tests/support/gpu.rs`.

  *Migration:* `RendererDevice::new(device, queue)` returns
  `Result<RendererDevice>`: add `?` (it fails with the new
  `FiggyError::PollThreadStartFailed` when the OS cannot start the thread).
  A native host that makes blocking calls only can build the bundle with
  the new `RendererDevice::blocking(device, queue)`, which starts no thread;
  an awaited wait on such a bundle fails with the new
  `FiggyError::AsyncWaitUnavailable` instead of never completing. A native
  caller that blocked on an export, a `resolve`, a prewarm, a window
  constructor or an adapter or device request with `pollster::block_on`
  can call the `_blocking` form instead and drop the executor dependency. `FiggyError` and `GpuPickError`
  gain the variants above, so an exhaustive `match` on them needs the new
  arms.

- **The renderer owns the column statistics** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `ColumnSource` and `HiLoColumnSource` no longer ask the host for `min()`
  and `max()`. The upload's `ColumnPairWriter` records the finite minimum,
  maximum and smallest positive value from the pairs as they are written, in
  the pass that already wrote them, so a source cannot report statistics
  that disagree with its values (a wrong `min` used to reach auto-fit and the
  bar validation's refusal of negative shares). A streamed source is measured
  the same way, one chunk at a time. `ColumnPairWriter::write_value(index,
  f64)` writes the `(hi, lo)` split and records the f64 itself, so a host's
  `0.3` fits an axis to `0.3`; `write_pair` records `hi + lo`, and a value
  the GPU reads as zero or infinite is recorded as it reads it. Each trait is now
  `len`, `is_empty`, `write_pairs(&self, &mut ColumnPairWriter)` and
  `write_pair_range(&self, start, &mut ColumnPairWriter) -> Result<(),
  ColumnRangeWriteError>` (default `Unsupported`). Removed: `min`, `max`, the
  legacy byte encoders `write_f32_le_into`, `write_f32_zero_lo_pair_le_into`
  and `write_f32_pair_le_into`, and `ColumnUploadStats`. Migration: rename
  `write_f32_pair_le_into_with_stats` to `write_pairs` and
  `write_f32_pair_range_into_with_stats` to `write_pair_range`, take the
  writer as `&mut`, and delete the statistics code, `min`, `max` and the
  legacy encoders. The browser wrapper no longer scans each array for its
  bounds before an upload, and the playground's 100-million-point column no
  longer evaluates its signal once more for them.
- **Names that say what they serve** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The `gpu_errorbar` module, which computes every series' GPU extents and
  not only error bars', is `gpu_extent`, and its error `GpuErrorbarError`
  is `GpuExtentError` (the error bar's own `GpuErrorbarExtent` and
  `GpuErrorbarExtentTicket` keep their names). `text_render::register_font_bytes`
  returns `renderer::Result` instead of a `String` error: bytes without a usable
  face are `FiggyError::InvalidFont`, an exhausted generation counter is
  `FiggyError::CounterExhausted`. Migration: `renderer::gpu_errorbar::` →
  `renderer::gpu_extent::`, `GpuErrorbarError` → `GpuExtentError`, and match
  `FiggyError` (or format it, as before) from `register_font_bytes`.
- **One replacement transaction instead of the demo loader** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Renderer::begin_load_demo`, which uploaded four fixed `demo_*` columns
  and replaced one chart as one commit, is
  `Renderer::begin_replace_chart_columns(chart_id, columns, prepare_state)`:
  any number of `(id, &dyn ColumnSource)` columns (new or replacing
  resident ones) and the chart's config and series, published together by
  `ChartColumnsReplacement::commit`, which returns the columns' handles in
  order; dropping the guard changes nothing. `RendererLoadDemo` is
  `ChartColumnsReplacement<'_, N>`. The sample data of `renderer::demo` and
  the model's document expansion (`expand`, `Expansion`, `ExpandedPanel`)
  are hidden from the documentation: machinery for the examples and the
  renderer, with no compatibility promise; the renderer no longer
  re-exports `expand`. Migration:
  `begin_load_demo(chart, &x, &sin, &t, &rc, f)` →
  `begin_replace_chart_columns(chart, [("demo_x", &x as &dyn ColumnSource),
  ("demo_sin", &sin), ("demo_t", &t), ("demo_rc", &rc)], f)`.
- **Device setup in `renderer::gpu`; the GPU internals hidden** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `create_instance`, `request_adapter_async`,
  `request_adapter_for_surface_async`, `request_device_async`,
  `create_surface_for_window`, `try_configure_surface`, `configure_surface`,
  `reconfigure_surface`, `render_clear` and `RenderOutcome` are the public
  `renderer::gpu` module. `data_render`, `gpu_contour` and `raster` are
  hidden from the documentation: they are the renderer's drawing internals,
  public for its own tests, with no compatibility promise; the column pool
  types a host names stay at the crate root. `WebDerivedSnapshot`,
  `WebDerivedStamp` and `Renderer::web_derived_snapshot`, which nothing
  read since the web wrapper stopped keeping its own caches, are removed.
  Migration: `renderer::data_render::{create_instance, request_adapter_async,
  request_device_async}` → `renderer::gpu::{..}`, and likewise for the
  surface helpers.
- **Every path checks the same rules** ([#160](https://github.com/shim9610/figgy/issues/160)).
  Registration, chart edits, the export of a held chart and the registered
  draw read the model's `validate_chart` / `validate_series`, and the
  host-owned `prepare` now refuses a series its polar axes do not draw, as
  the other paths always did. The renderer itself checks only what needs
  its column pool. Errors keep their variants (`InvalidChartArea`,
  `InvalidConfig`, `InvalidSeriesConfig`); a chart-wide series rule now names
  the series that breaks it (the first whose addition to the list fails the
  rule) rather than the first series of its kind. `ConfigError` and the
  `validate` module are re-exported.
- **One export API** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The eighteen `export_panel_*` / `export_document_*` methods (RGBA, PNG,
  SVG, PDF, each with and without a clear colour) are four:
  `Renderer::export_rgba`, `export_png`, `export_svg` and `export_pdf`, each
  taking an `ExportSource` and `ExportOptions`. `ExportSource::Chart {
  chart, series }` (or `ExportSource::chart(&chart, &series)`) exports what
  the caller holds; `ExportSource::Registered(id)` exports a registered
  chart's stored config and series, so a host no longer clones them to
  export. `ExportOptions { scope, scale, clear }` picks the panel or the
  whole document (`ExportScope`), the scale and the background
  (`ExportOptions::panel(scale)`, `ExportOptions::document(scale)`,
  `.with_clear(color)`; transparent by default). A vector export renders the
  whole document and refuses `ExportScope::Panel` with
  `FiggyError::InvalidConfig`. `RadialRenderer` has the same four,
  `export_rgba(chart, size, options)` and so on; its exports now honour
  `options.clear` instead of always clearing to transparent. Migration:
  - `export_panel_rgba_async(&chart, &series, s)` →
    `export_rgba(ExportSource::chart(&chart, &series), ExportOptions::panel(s))`;
    `export_document_*` → `ExportOptions::document(s)`;
  - `*_with_clear_async(.., c)` → `.with_clear(c)` on the options;
  - `*_png_bytes_*` → `export_png`, `*_svg_*` → `export_svg`,
    `*_pdf_*` → `export_pdf`;
  - `RadialRenderer::export_rgba_async(&chart, size, s)` →
    `export_rgba(&chart, size, ExportOptions::document(s))`, and likewise
    `export_svg_async` / `export_pdf_async`.
- **Every `Renderer` call reports its own GPU error; `WindowedRenderer` keeps only the surface** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The trailing device-health check that only `WindowedRenderer` ran after
  each call (#158) now runs inside `Renderer`'s own methods, so a host that
  embeds `Renderer` directly (egui, iced) gets a call's GPU error from that
  call, not from the next one. `WindowedRenderer` keeps what the surface
  owns (`resize`, `draw`, `draw_registered`, `draw_prepared`,
  `first_frame_ready`, `warm_up`, `pick_chart_at`, `pick_chart_data_at`,
  `surface_size`, `surface_format` and the constructors) and implements
  `DerefMut<Target = Renderer>` beside `Deref`; its 87 hand-written
  forwarding methods are gone. Migration: none for calls written as
  `windowed.method(...)`, which now resolve to the `Renderer` method through
  auto-deref; code that named a forwarder as a path
  (`WindowedRenderer::add_column`) names the `Renderer` method instead.
  `Renderer::ensure_target_format` changes the pipelines' format only;
  `WindowedRenderer::resize` sets it back to the surface's.
- **Radial exports are guarded like every other export** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `RadialRenderer::export_rgba_async`, `export_svg_async` and
  `export_pdf_async` check the device health before and after the export,
  render inside out-of-memory, validation and internal error scopes (around
  the whole render on native, from the first resource to the readback in the
  browser), and attach the completion of their transient target and readback
  buffer through `end_frame`. A GPU error during a radial export now fails it
  with `RadialError::Gpu` instead of surfacing on a later call. The radial
  constructors follow `Renderer::try_new`, which they always were in effect
  (they return a `Result`). Migration: `RadialRenderer::new` is
  `RadialRenderer::try_new`, and `RadialRenderer::new_async` is
  `RadialRenderer::try_new_async`; the arguments are unchanged.
- **One GPU ledger and budget per device bundle** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `RendererDevice::new` creates the ledger, and every `Renderer`,
  `WindowedRenderer` and `RadialRenderer` built from the bundle or its clones
  charges it and spends its one budget. `RendererDevice::ledger` returns it.
  The column pool charges its slab, backup, growth candidates and upload
  staging to the ledger as it creates them, instead of counting its own
  bytes beside it, so a budget check sees every pool on the device.
  `RadialRenderer::end_frame` retires through the same path as
  `Renderer::end_gpu_frame`. Behaviour changes: `gpu_memory_usage` and the
  budget cover the whole bundle, so two renderers built from one bundle report
  the same total; `RadialRenderer` starts with no budget instead of 256 MiB.
  Migration:
  - `GpuAllocCtx` is `{ device, queue, ledger }`, built with
    `GpuAllocCtx::new(device, queue, ledger)`; pass `gpu.ledger()` or an
    `Arc<GpuLedger>` of your own. `GpuBudget` and `GpuAllocCtx::unbudgeted`
    are removed: the pool reads the budget from the ledger.
  - `ColumnPool::retired_bytes`, `clear_retired_bytes`, `peak_bytes`,
    `reset_peak_bytes` and `buffer_creations` are removed. Read the
    `GpuResourceKind::ColumnPool` row of `GpuLedger::snapshot()` and
    `GpuLedger::peak_bytes` / `reset_peak`.
  - `GpuMemoryUsage::with_kind`, `with_creations` and `with_peak_at_least`
    are removed; build a report from a `GpuLedger`.
  - `RadialRenderer::set_memory_budget` takes `Option<u64>` and returns the
    current total when the new ceiling is already below it, as
    `Renderer::set_memory_budget` does; `RadialRenderer::memory_budget` reads it.
  - `GpuErrorbarExtentEngine::new` and `new_observed` take a
    `&RendererDevice` and use its ledger and health record;
    `new_tracked` and `new_observed_tracked` are no longer public.
  - `GpuLedger` gains `budget`, `budget_limit` and `set_budget`.
- **The column registry is readable** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Renderer::logical_column_ids` lists every column a host registered,
  streamed or resident (record and ragged member columns excepted), and
  `Renderer::column_revision(id)` returns a streamed column's declared
  revision or the allocation epoch of a resident column's latest upload, so a
  host reads the registry from the renderer instead of keeping its own.
- **Registered charts drawn on a display** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Renderer::set_chart_display(chart, Some((width, height)))` makes
  `draw_registered` draw the chart scaled and letterboxed onto that surface,
  as `display_config_for_surface` places it, with the series' styles at the
  display scale; `None` draws the document as stored. The stored config is
  unchanged, so a fit token stays current. The placed config and the styles
  are rebuilt only when the config, the series or the surface change.
  `chart_display_config` returns the config, panel rect and scale it draws
  with, and `preflight_chart_styles` checks a series list's styles against the
  memory budget and the device's storage limits without creating them.
  Behaviour change: `draw_registered` skips a series whose source is not
  resident (a cancelled or unpublished stream) instead of failing the frame,
  and draws the rest of the chart. A 3D chart's point picks use the styles
  at the display scale when the pick is made in that display.
- **Whole-chart auto-fit on the renderer** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `Renderer::begin_auto_fit_all(chart, padding)` returns an
  `AutoFitAllTicket`; `AutoFitAllTicket::resolve` awaits its reductions
  without borrowing the renderer, and `Renderer::commit_auto_fit_all`
  applies the result unless an axis, the fit geometry or a source column
  changed in between (`StaleStateToken`). It fits the active layer as the
  browser product did: a ternary layer is left as it is, a parallel chart
  fits each axis to its columns, histograms fit from their columns'
  statistics, every other series from its drawn GPU extent (moved by its
  line offset), and a 3D layer's z axis to its height columns; the
  annotation layer fits the base axes. Each series' extent is kept with the
  chart and reused while the series' columns are unchanged, so a repeated
  fit submits no reduction. `WindowedRenderer` has the same two methods.
  Native hosts get the whole-chart fit the browser product had alone.
- **No blocking wrappers** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The fourteen native wrappers that only ran `pollster::block_on` over an
  async method are removed: `Renderer` and `WindowedRenderer`
  `export_panel_rgba`, `export_panel_png_bytes`, `export_document_rgba`,
  `export_document_png_bytes`, `export_document_svg` and
  `export_document_pdf`; `Renderer::for_window`; `RadialRenderer`
  `export_rgba`, `export_svg` and `export_pdf`;
  `RadialPickTicket::resolve_blocking`; and `data_render::request_adapter`,
  `request_adapter_for_surface` and `request_device`. `pollster` is no
  longer a dependency of the renderer. Migration: call the `_async` method
  and block on it with the executor of your choice, for example
  `pollster::block_on(renderer.export_panel_rgba_async(&chart, &series, 1.0))`
  or, with `use pollster::FutureExt as _;`,
  `renderer.export_panel_rgba_async(&chart, &series, 1.0).block_on()`.
- **Font lookups without a lock** ([#160](https://github.com/shim9610/figgy/issues/160)).
  Registered fonts are an immutable snapshot published through `arc-swap`:
  `register_font_bytes` builds the successor under the registration mutex,
  and family lookups, per-character fallback and `font_family_available`
  read the current snapshot without a lock. A registered file is copied once
  and its faces are parsed at registration; before, every resolved face of a
  file leaked a second copy of the whole file. System faces are copied once
  per font file on first use, so the faces of one collection (`.ttc`) share a
  copy, and the shared system-face mutex is gone. A family lookup no longer
  allocates a key on every character. No API change.
- **Plain wgpu handles instead of `Arc`-wrapped ones**
  ([#160](https://github.com/shim9610/figgy/issues/160)). `wgpu::Device`,
  `wgpu::Queue` and `wgpu::Buffer` are reference-counted handles that
  implement `Clone` and compare by identity, so wrapping them in `Arc` counted
  every reference twice. `RendererDevice::new` takes `wgpu::Device` and
  `wgpu::Queue`; `RendererDevice::device()` / `queue()` and
  `Renderer::device()` / `queue()` return `&wgpu::Device` /
  `&wgpu::Queue`; the public buffer fields `ArcScratch::arc`,
  `ColumnLineLayer::arc` and `ValueLabelData::params` / `cells` are
  `wgpu::Buffer`. The web chart drops
  an `alive` flag that could never be false, and the radial layout drops a
  boxed iterator. Migration: pass the device and queue themselves, as
  `RendererDevice::new(device, queue)`, instead of `Arc::new(device)` /
  `Arc::new(queue)`; clone a handle with `.clone()` where `Arc::clone` was
  used, and compare handles with `==` where `Arc::ptr_eq` was used.
- **A call reports its own GPU errors**
  ([#158](https://github.com/shim9610/figgy/issues/158)). Every
  `WindowedRenderer` call that records GPU work checks the device health at
  its end, as `draw` already did. So does each `begin_*_extent` on a column
  upsert guard, and each pick or extent ticket's `resolve` after its final
  unmap. On native, a GPU error a call raised is returned by that call
  rather than by the next one. None is left for `Drop`, which has no caller
  to report to; it now ignores only a device lost after the last call. In a
  browser, WebGPU reports errors asynchronously: a host that needs the last
  call's error awaits `wait_submitted_work` before dropping the chart.
  Behaviour change: an error that used to surface on the following call now
  surfaces on the call that caused it. No API change.
- **No panic paths**
  ([#152](https://github.com/shim9610/figgy/issues/152)). Product code has
  no `unwrap`, `expect`, `panic!`, `unreachable!` or assert macro, and
  every error is propagated; `python3 ci/verify.py static` runs a clippy
  gate (`clippy.toml`) that fails on any of them. GPU work that can fail
  runs inside error scopes on native (`gpu_checked`), and
  `RendererDevice::new` registers a device-lost callback and an
  uncaptured-error handler, so a lost device or a validation error surfaces
  as `FiggyError::DeviceLost` or `FiggyError::GpuValidation` from the next
  call instead of a wgpu panic. A panic inside a host `ColumnSource` is
  contained and reported as an allocation error; an out-of-range write is
  recorded and reported instead of panicking. Every index into an array
  produced elsewhere is backed by a test or a build-time check at the
  array's producer. A failed source write into a reused stream staging slot
  no longer unmaps the slot twice, and a panel-scope export frame keeps one
  panel configuration per item.
  Memory: the glyph mask cache is held to 64 MiB and the glyph metrics cache
  to 65 536 entries, evicting the least recently used, and a glyph is
  rasterized at most 4096 px tall.
  Breaking changes and their migration:
  - `RendererDevice::wait_idle`, `wait_submitted_work` and
    `service_gpu_completions` return `Result`; callers add `?`.
  - `FiggyError` gains `Internal { invariant }`, `GpuValidation { resource,
    reason }` and `DeviceLost { reason }`; `AllocError` gains
    `Internal { invariant }`, `Gpu { resource, reason }` and
    `MemberColumn`; `GpuPickError` gains `Internal` and `Gpu`;
    `GpuErrorbarError` gains `Internal { invariant }` and `Gpu`. Exhaustive
    matches add the arms.
  - `RaggedRef::values` and `offsets` return `Option<ColumnHandle>`.
  - `charged_buffer` and `charged_buffer_init` return `Result`.
  - `draw_rich_text`, `draw_axes`, `draw_grid_layer` and
    `draw_decoration_layer` return `Result`.
  - `shader_assembly::module` returns `Option<&'static str>` for an unknown
    name.
  - `raster_chart_to_rgba` is removed; use the renderer's PNG export.
- **Shape annotations**
  ([#138](https://github.com/shim9610/figgy/issues/138), #58). The
  annotation raster and the vector export draw the shape kinds through the
  new `Sink::polygon` (`CanvasSink`: `Canvas::fill_polygon` and the
  stroker's `stroke_polygon_outline`, one closed path so a dash runs around
  the corners; `VectorSink`: a closed path with fill and stroke); the
  selection box strokes a part's outline as its own point list; the
  annotation map gains `hit_vertex` and `drag_vertex`. The `Annotations`
  catalogue scene gains a rectangle below the data, an ellipse and a
  concave polygon.
- **Roses: category bars on a polar chart**
  ([#133](https://github.com/shim9610/figgy/issues/133), #58). A category
  bar series on a polar chart draws each bar as the wedge of its category's
  angular slot between the stack's start and end radii (`category_wedge`
  in `common/category_bars.wgsl`, `CategoryBarParams.wedge`), the group
  layout, stack arithmetic, overrides, materials, corner rounding, value
  labels (`value_label_wedge`), pick, highlight and `step_selection` the
  bar's; the wedge's footprint is the radial chart's sector
  (`radial_sector.wgsl`, now spliced into the bar module) built in
  registers, so nothing is stored per bar. Grouped wedges sit side by side
  in the categories' order, `bar_gap` pixels apart along the ring. The
  `Rose` catalogue scene is a wind rose of sixteen directions and three
  stacked speed bands.
- **Parallel coordinates**
  ([#141](https://github.com/shim9610/figgy/issues/141), #58). A
  `Parallel` series draws through the line module's new
  `parallel_columnar.wgsl` entries: `vs_parallel` / `fs_parallel` draw one
  stroke quad per segment instance with no vertex slot, both ends read
  from the column pool through the series' axis lane table and placed by
  its axis range table (group 3: the point value map, the pool,
  `ParallelParams`, the lane table and the range table;
  `create_parallel_data_bind_group_layout` /
  `create_parallel_data_bind_group`, `ParallelParamsGpu`,
  `ParallelAxisGpu`), the normalization computed on every draw and never
  stored; a row's colour is its colour column's value through the
  colourbar ramp or the series colour; `vs_parallel_selected` redraws the
  picked row in the selection's style over every body; `pick_parallel`
  answers the nearest segment's row as a point (`GpuDataPickGeometry::Parallel`,
  `DataPickPipelineBundle.parallel`, `DataPickShaders.line`).
  `TargetPipelines.line_parallel` / `line_parallel_selection`,
  `ColumnParallelLayer` / `ParallelSelection` / `draw_parallel_layer` /
  `draw_parallel_selection`, `SeriesLayers.parallel`,
  `EffectiveSeriesPrimitives.parallel`, the parallel layout in
  `Renderer`, `PrepareContext` and the pick bundle; the resident display
  key carries `Config::parallel`. `parallel_axes.rs` draws each axis's
  line, ticks (`paxis_{i}`, `ptick_{i}_{k}`) and tick labels in `left_y`'s
  style over its range; `Element::ParallelAxisLabel(i)` places them, the
  chart's `left_y` and `right_y` draw nothing, and the grid is not drawn.
  `Chart::auto_fit_parallel_axes` fits each axis's range from the columns'
  statistics (`auto_fit_layer_union` on a parallel chart too); the x and y
  fits leave a parallel chart alone. Registration validates the model's
  rules, every axis column and the colour column's colourbar; a polar
  chart and a streamed source are rejected (`STREAMED_PARALLEL_REASON`).
  `series_draw_info` reports the rows' segments. The `Parallel` catalogue
  scene is 150 iris-like flowers over four axes coloured by species.
  Migration: `ensure_precise_variants_for_items`, `prewarm_all_observed`
  and `DataPickPipelineBundle::new_observed` take the parallel layout, and
  `DataPickShaders` the line module; an exhaustive `DataRenderType` match
  adds the arm.
- **Sunbursts** ([#140](https://github.com/shim9610/figgy/issues/140), #58).
  The radial layout lays a sunburst's rings as equal bands between the hole
  and the rim (`ring_band`), a node's sector the existing `Sector` with its
  ring's outer radius and inner fraction, so the shader, the pick, the
  highlight and the hover are unchanged; the sector buffer holds the rings
  in order, the material seed is `(ring · 97 + index) · 13.17`. Outside
  labels are the outermost ring's, an inner ring's `Outside` draws inside
  its band (`placement`); every node's label is a decoration. The
  `Sunburst` catalogue document is a three-ring budget, swept by the oracle
  and the vector export.
- **Colour images (imshow)**
  ([#139](https://github.com/shim9610/figgy/issues/139), #58). An
  `RgbImage` series draws through the field module with `FIELD_FLAG_RGB`:
  the three channel grids share one `GridColumn` table (the green and blue
  tables `FieldParamsGpu.image[0]` entries after the red one, read through
  `grid_value_in`), and `fs_main` paints `field_image_color` (each channel
  sampled flat or bilinearly, mapped from the `range` pair and clamped,
  premultiplied by `opacity`) instead of the ramp; a cell with a
  non-finite channel is unpainted and not picked (`field_image_paints`).
  No colourbar is read; the cell pick, the selection outline and
  `step_selection` are the heatmap's. `FieldParamsGpu` grows from 64 to
  80 bytes (`image: [u32; 4]`). Registration validates the three grids'
  columns and the model's rules; a polar chart and a streamed source are
  rejected (`STREAMED_IMAGE_REASON`). The `Image` catalogue scene is a 256
  by 192 test card the host composes as bytes, with markers over it.
- **Hexagonal grids (hexbins)**
  ([#134](https://github.com/shim9610/figgy/issues/134), #58). A heatmap
  on a `Hexagonal` grid draws its cells as the Voronoi cells of their
  centres in the data metric (`field_grid.wgsl` `hex_cell`, the nearest
  centre among the nearest column, its nearest row and the six neighbours;
  `FIELD_FLAG_HEXAGONAL` in `FieldParams.flags`), and the fill, the cell
  pick and the selection outline (`hex_edge_distance_px`) all go through
  that one lookup, O(1) per fragment with nothing stored per cell. The
  pool measures every column's uniform step as it is written
  (`ColumnSlot::uniform_step`, `data::UniformStep`; a promoted streamed
  column has none), and registration requires it of a hexagonal grid's
  coordinate columns besides the model's rules; streamed sources are
  rejected (`STREAMED_HEXAGONAL_REASON`). The series fit reaches past the
  centres-cells bounds by the hexagons' overhang
  (`GpuSeriesExtentTicket`, a sixth of the column pitch each side and half
  a row pitch past the raised columns). The `Hexbin` catalogue scene bins
  a bivariate normal on the host.
- **f64 columns upload exactly.** `Column<f64>` and `Column<Option<f64>>`
  registered through `ColumnSource` (`add_column`) wrote `(v as f32, 0)`, so
  every bit past f32 was lost unless the host used `add_hilo_column`; past
  2^24 that silently moved ragged group offsets. They now write the
  `(hi, lo)` split (about 48 significant bits; integers below 2^48 exact)
  straight into the pair writer, resident and streamed alike
  (the maintainer's exception to rule 8 for upload precision, #107), and the
  web wrapper's borrowed f64 column does the same. No extra pass, buffer or
  copy; memory is unchanged (every lane was already a pair). Values that are
  not exact f32 now place a sub-ulp closer to their f64 position.

- **No export scale cap.** `MAX_EXPORT_SCALE` (8.0) is removed and
  `clamp_export_scale` only raises a scale to `MIN_EXPORT_SCALE` (0.25); the
  radial chart's 0.5..4 range keeps only its lower bound. The scaled target
  must fit the device's texture limit (`max_texture_dimension_2d`); past it
  the PNG, SVG and PDF exports fail with `FiggyError::GpuResourceLimit`
  before anything is allocated, as a target past the limit already did.
  Streamed charts keep their 8× bound (streaming is not extended).
  Migration: code that names `renderer::MAX_EXPORT_SCALE` picks its own
  bound; code that relied on a scale above 8 being clamped now gets that
  scale, or the `GpuResourceLimit` error when it does not fit the device.

- **No value label cap**
  ([#107](https://github.com/shim9610/figgy/issues/107) R11). The value
  label uniform held 64 label cells (`ValueLabelParamsGpu::cells`, 1120 B)
  and the atlas 64 widths, so a bar, box or radar series over more
  categories labelled its first 64 only. The cells are now a read-only
  storage array bound at group 1 binding 7 of the value label draw and
  pick, one per label, written straight into its buffer's mapping at
  prepare; the atlas keeps its bake's cell widths. Every category is
  labelled, drawn, picked and dragged: 300 labels in `value_labels_render`.
  No policy cap replaces it: the label count is the host's to budget.
  Images of up to 64 labels are byte-identical; a series over more
  categories now also draws the labels past the 64th. Migration: the public
  `VALUE_LABEL_CAPACITY` is removed; `ValueLabelParamsGpu` loses `cells` and
  is 96 B; `ValueLabelData` gains `cells`; a custom value label layout adds
  binding 7 (read-only storage).

- **No series cap on bar, box and radar groups**
  ([#107](https://github.com/shim9610/figgy/issues/107) R3). A category
  bar group kept its members' value column lanes in a 16-entry table in
  the params uniform (`CategoryBarParamsGpu::values`, 112 B). The lanes are
  now the series' lane table, a read-only storage array bound at group 3
  binding 10 beside the pool and the params and sized by what it holds: a
  bar group's members, a box plot's 13 record and outlier lanes, a radar
  label's value column. The table is written straight into its buffer's
  mapping at creation; nothing is staged on the CPU. A group of 40 series
  stacks and picks like a group of two; the stack and percent loops read
  every member at each bar, so a draw costs `O(members² · categories)`
  pool reads, measured in `category_bars_render` and documented in
  SHADER_COMMON §5.13, never capped. The renderer's 16-member check is
  gone; existing category bar, box plot and radar images are
  byte-identical. Migration:
  `CategoryBarParamsGpu` loses `values` and is 48 B;
  `create_category_bar_data_bind_group` takes the lane count and a writer
  (`lanes, write: impl FnOnce(&mut dyn FnMut(u32)) -> Result<()>`) before
  the style map and returns `Result<CategoryBarData>`; a custom layout
  built for the category bar group 3 adds binding 10 (read-only storage).

- **Column-backed series past the old caps draw and pick**
  ([#107](https://github.com/shim9610/figgy/issues/107) R5, R10). A box
  plot's override rows were built in a stack table of 64 categories' rows
  and the rows past it were dropped; they are now written straight into
  the override buffer's mapping at creation, sized by the series'
  overrides. A layer's pick kept its
  line offset members' transforms in a 32-entry array and skipped members
  past it; it now keeps the members' panels from the expansion and the
  engine asks for each member's transform. Migration: none.

- **No ragged value cap**
  ([#107](https://github.com/shim9610/figgy/issues/107) R6). A ragged
  column's offsets were `(hi, lo)` f32 pairs read as `u32(hi + lo)`, exact
  only to 2^24, so a ragged column (a box plot's outliers) held at most
  2^24 values. The pool now stores the offsets as `u32` index lanes: the
  integer `hi + lo` of the pair the source writes, summed in f64, as `u32`
  bits in the pair's first lane and 0 in the second, read with
  `bitcast<u32>` (SHADER_COMMON §5.14). Every offset up to `u32::MAX` is
  exact: the encoding is tested on the CPU write and a GPU read-back at
  2^24 - 1, 2^24, 2^24 + 1, `u32::MAX - 1` and `u32::MAX`, where the
  former read loses 2^24 + 1. The only bound left is the `u32` index space
  (`u32::MAX` values); box plot images are byte-identical. An offset is the
  integer its source writes, so offsets past 2^24 come from a source that
  writes the `(hi, lo)` split, as `Config::from_boxplot`'s uploads, the
  WASM `register_ragged` and now `Column<f64>` (see "f64 columns upload
  exactly") do. Migration: the unreleased `MAX_RAGGED_VALUES` is removed.

- **The picker measures in the draw's own pixel frame**
  ([#114](https://github.com/shim9610/figgy/issues/114)). The exact
  picker projected every vertex to canvas pixels with its own function
  while the draw projects through `data_to_ndc / pixel_to_ndc`, and the two
  rounded apart: a step riser of 1e-5 px had zero length in the pick and
  its square caps in the draw, so the caps' dash was painted but not
  picked. The new shared `data_to_px` (projection.wgsl) is that projection
  in one place: the line vertex stages and the arc scan call it, and the
  exact pass measures every primitive in its frame (`pick_data_px`, with
  the cursor mapped in once by `pick_cursor_px`). The sketch stroke's pick
  also stopped clamping its knot weight, since a multisampled fragment
  past a strip's square cap extrapolates its end span to the pixel centre
  (a faint dash sliver at a joint was painted but not picked). New
  property tests draw seeded random compositions of elementary functions
  under random styles, dashes, widths, step modes, gaps and off-screen
  leads (`random_composite_lines_pick_where_they_draw`,
  `random_composite_markers_and_star_lines_pick_where_they_draw`), which
  found both. Migration: none.

- **Line arcs stay exact past 2^24 px**
  ([#107](https://github.com/shim9610/figgy/issues/107)). The arc prefix of
  a dashed, sketched, Milkyway or streamline line held one f32 per point, so
  past 2^24 px of drawn arc (16.7 M px) its positions rounded to whole
  pixels and beyond: a dashed line drew solid, and the sketch wobble, the
  Milkyway clump noise, the star slots and the streamline arrowheads moved
  to the rounded positions. Each point's arc is now a 64-bit fixed-point
  integer of 1/65536 px held as two `u32` words, summed exactly with integer
  adds (SHADER_COMMON §5.6 arc pairs). Every reader takes a segment's arc as
  the pair at its start plus a small offset, and reduces the start exactly
  (integer long division) for the dash phase, the noise cell and the slot
  index, so the pattern far along the arc is the pattern at its start:
  `dashes_keep_their_pattern_past_two_to_the_twenty_six_pixels_of_arc` drew
  a solid stroke before and draws [8, 4] dashes now, and
  `gpu_arc_prefix_stays_exact_past_two_to_the_twenty_four_pixels` holds a
  3.9e7 px prefix to 0.05 px of the f64 sum. Picking reads the same pairs,
  and the streamed arc replay writes them too (upload precision, the
  maintainer's exception to rule 8). The prefix is 8 bytes per point
  instead of 4; a dash period, noise wavelength or slot pitch is rounded to
  1/65536 px. The star pass skips slots its existence gate drops before
  any arc arithmetic. Migration: a custom line pipeline binds the arc
  slots 4/5 as `Uint32x2` with an 8-byte stride, and a custom reader of
  the prefix decodes `(low + high * 2^32) / 65536` px.

- **No star slot cap**
  ([#107](https://github.com/shim9610/figgy/issues/107) R7). The arc scan's
  `star_indirect` kernel capped the candidate slots of a Milkyway star pass
  and a streamline's arrowheads at `STAR_MAX_SLOTS` (2 000 000) per series,
  so a series whose drawn arc ran past 1 M px (at the default 0.5 px pitch)
  drew no stars or arrowheads past that point. Every slot of the arc now
  draws, up to the `u32` instance count a draw can name. The cost grows with
  the drawn arc: one 6-vertex instance per slot, each a binary search over
  the arc prefix (`O(slots · log n)`), measured in
  `milkyway_stars_draw_past_two_million_arc_slots` and documented in
  SHADER_COMMON §5.6, never capped. Images of series under the old cap are
  byte-identical. Migration: the public `line_arc::STAR_MAX_SLOTS` is
  removed.

- **The column pool grows by default, to the device's limits**
  ([#107](https://github.com/shim9610/figgy/issues/107) R8, R9). A pool
  kept the capacity `Renderer::try_new` was given unless the host chose
  `GrowthPolicy::OnAllocFailure`, the web wrapper's pool stayed at 16 MiB
  unless `set_pool_auto_growth(true)` was called, and `request_device`
  opened the device with the WebGPU baseline limits (128 MiB storage
  binding, 256 MiB buffer, 8192 px textures) whatever the hardware
  allowed. `GrowthPolicy::OnAllocFailure` is now the default, natively and
  on the web, and `request_device` / `request_device_async` request the
  adapter's supported limits. A column registers unless the device or the
  memory budget cannot hold it, which is the only failure rule 10 allows.
  The pool's starting capacity is still the one the host passes.
  Migration: a host that relied on `OutOfSpace` from a full pool calls
  `set_pool_growth_policy(GrowthPolicy::Fixed)` (web:
  `set_pool_auto_growth(false)`); growth relayouts the pool, which
  invalidates live `ColumnHandle`s and `PreparedFrame`s exactly as a
  defragmentation does. A host that opens its own device keeps its own
  limits.

- **Streamlines** ([#51](https://github.com/shim9610/figgy/issues/51), #58
  item 22b). A `Streamlines` series draws its polylines through the line
  path (dashes, picking and the picked ring included) and its arrowheads
  through the vector glyph (rule 9): the line's arc prefix is scanned with
  the arrow spacing as the slot pitch, the scan's indirect kernel counts the
  slots, and the new path input of the vector shader
  (`VectorInput::Path`, `VECTOR_PATH_SHADER`, `vector/vector_input_path.wgsl`)
  places a head centred on each slot at `(k + 0.5) * spacing` of drawn arc,
  along the segment's tangent, through `common/arc_walk.wgsl` (the star
  pass's slot search, moved there unchanged) and `common/path_arrow.wgsl`.
  The heads keep their pixel spacing under pans, zooms and every export
  scale, skip NaN gaps and draw under the stroke. A colour column draws the
  stroke through the new mapped line pipeline (`vs_main_mapped` /
  `fs_main_mapped`, the value column in vertex slots 6/7, the point value
  map as group 3) and colours the heads through the vector colour map; the
  value is interpolated along each segment. Heads pick with their line
  (`PICK_VARIANT_PATH_ARROWS`), naming the segment's nearer endpoint.
  Precise draw style only; polar charts and streamed sources are rejected
  with `InvalidSeriesConfig`. Placement is in data space: exact on linear
  axes, a sub-pixel off the drawn chord on a logarithmic one, the pick
  following the draw. The point value map layout is now visible to the
  fragment stage. Migration: an exhaustive `match` on
  `data_render::vector::VectorInput` needs a `Path` arm;
  `ColumnVectorLayer` literals gain `indirect: None`, `ColumnLineLayer`
  literals `value_map: None`; `VectorParamsGpu::pad0` is now `spacing_px`
  (zero for the point and grid forms).

- **Violins and rainclouds** ([#53](https://github.com/shim9610/figgy/issues/53)).
  The bar module draws a box plot's violins and raw points through its
  violin fragment (`common/violin.wgsl`, SHADER_COMMON.md §5.19), each with
  a group 3 of its own over the box group's layout, under and over the
  boxes: one instance per curve segment, finding its category by a binary
  search over the group offsets, so a long curve costs only its own
  segments, and one instance per raw point. A pixel belongs to the segment
  whose value range holds its snapped centre, so the segments tile without
  seams under multisampling. The violin's data pick (`pick_violin`) tests
  each category and half once with a binary search inside it and answers
  the category as a histogram bin; the raw points are picked by the point
  picker through the exact variant `PICK_VARIANT_CATEGORY_JITTER` (256),
  placed by the shared fragment `common/category_jitter.wgsl` (§5.20) that
  the draw places them with, the gates reaching one and a half slots past
  a point's category. `box_slot` now places members through that
  fragment's `category_member` (the same arithmetic, so boxes, range bars
  and category bars draw as before). Box plot lane tables grow to 18
  entries (the box width and the violin curves the value label clears);
  `PickQueryParamsGpu` grows from 528 to 560 bytes with the raw points'
  layout. A box override restyles its category's violin. The fit covers
  the curves and the points' values; streamed sources are rejected.
  Two playground scenes (Split violins, Raincloud) join the decoration
  oracle's catalogue. Picking among 600 violins of 4 096 curve points takes
  about 4 ms on lavapipe, and a raw point among 3 000 000 about 70 ms.
- **3D surfaces** ([#49](https://github.com/shim9610/figgy/issues/49)).
  A 3D panel's composite gains a `Depth24PlusStencil8` attachment at the
  target's sample count, created only for that panel on screen
  (`ResidentDisplay`) and in export; the 2D pipelines and the shared frame
  and export passes are unchanged. The surface module
  (`data_render/surface_mesh/`: `surface_input_grid.wgsl`,
  `surface_entry.wgsl`, with `common/project_3d.wgsl`) draws one instance
  per quad from the field group's grid table, its positions, colours and
  normals computed on every draw; `field_paint.wgsl` moves the heatmap's
  colour code to `common/` so the surface paints as the heatmap does. The
  field module gains `vs_floor` and `fs_floor_contour_pick` for floor
  contours, drawn before the surfaces without writing depth. Picking is a
  1×1 ID pick (`renderer/surface_pick.rs`, an `Rgba32Uint` target) with the
  depth test, so a hidden sample is never answered; the picked sample's
  patch is drawn in the live highlight slot over its occluders. The box
  grid, axes, ticks, labels and titles are emitted through the sinks
  (`axis_render/box3d_axes.rs`). `Chart::auto_fit_view_3d_z` fits
  `right_y` to the height columns' statistics. Annotations place ratio
  anchors only, streamed sources are rejected, and the compile capture
  records `depthStencil`. A grid past `1 << 22` quads draws whole.
- **A 3D series' height column is enumerated with its columns**
  ([#207](https://github.com/shim9610/figgy/issues/207)). Removing a
  series' `z_column` now removes the series, as removing any column it
  reads does, instead of failing at the next prepare with
  `UnknownColumn`. A streamed height column is refused with
  `ColumnNotResident`. Pick invalidation, the derived-data stamps and the
  browser's referenced columns read the same list. `series_draw_info`
  counts the height column on a 3D panel, so it reports the rows the draw
  clamps to.
- **3D scatter and line series** ([#132](https://github.com/shim9610/figgy/issues/132)).
  A scatter, line or scatter-line series with a `z_column` draws on a 3D
  chart through new entries of the scatter, line and arc modules (`vs_3d`,
  `vs_3d_mapped`, `vs_3d_pick`, `vs_3d_mapped_pick`, `vs_pick_ring_3d`,
  `vs_pick_ring_3d_mapped`, the line's `vs_3d` / `vs_3d_pick`, the arc
  scan's `seg_init_3d`; `common/project_3d.wgsl` gains `view3d_point_t`,
  `view3d_point_valid` and `view3d_pick_id`): markers are camera-facing
  quads of their pixel size at the depth of the front of a ball of their
  radius, strokes keep their pixel width between the projected ends at the
  front of a tube of their half width with the dashes measured along the
  projected chord (`project_box_front`), both depth tested against the
  surfaces and each other with alpha to coverage on a multisampled composite.
  They are drawn into the panel composite after
  the surfaces (`SeriesLayers::line_3d` / `scatter_3d`), picked by the 1×1
  ID pick (kind 1: `PickedData::Point`, the marker or the segment's nearer
  end, decided on the whole chord since the pick's 1×1 target clips the
  stroke around the cursor, never a mark behind a surface) and ringed in
  the live slot. Eight
  pipelines (`TargetPipelines::view3d`) and one compute pipeline, and one
  bind group layout for the mapped markers (the point value map and the
  camera in one group, `surface_mesh::create_view3d_value_map_bind_group_layout`;
  `ValueMapScratch` keeps its buffers for it). The `Surface` catalogue
  scene gains a trajectory. A trajectory past `1 << 20` rows draws whole
  and picks.
  Guide: `SURFACE.md`.
- **Ternary region fills** ([#49](https://github.com/shim9610/figgy/issues/49),
  [#55](https://github.com/shim9610/figgy/issues/55)). A ternary panel with
  a region fill composites with the same `Depth24PlusStencil8` attachment,
  on screen and in export. The region module
  (`data_render/ternary_region.wgsl`, SHADER_COMMON.md §5.22) fills by
  stencil-then-cover with the even-odd rule: `vs_stencil` draws one
  triangle per edge from the triangle's centre, `vs_cover` paints the
  triangle once per series where the stencil is odd. A ring's closing
  edge reads its first vertex's row from the ring-start scan, three compute
  entries of the same module in the arc scan's pattern
  (`ternary_region.rs::scan_ring_starts`): each row's start is the running
  maximum of the rows after invalid rows, scanned in blocks of 256 and
  levels, written once per draw and pick into one `u32` per row. No
  invocation walks its ring, so the work per vertex is bounded whatever
  the ring's length. A walk to the ring's first vertex in one invocation
  had made Metal kill the command buffer for one ring of 131 072 vertices
  and leave the device drawing nothing (#63). Group 1 binds the pool as
  storage with the fill's params and the ring starts; no vertex slot.
  The point picker counts the crossings of the cursor's ray with the edges
  under `PICK_FLAG_REGION` and folds the parity in the per-series reduction
  (`PICK_KIND_REGION` entries beside the workgroup candidates): an odd
  count is a fill hit at distance zero on the lowest crossing edge's start,
  losing every tie to a mark. Its closing edges read the same scan, bound
  as the exact pass's group 2 binding 1 (`pick_ring_start`) beside the arc
  prefix, so the exact layout binds eight storage buffers. The compile
  capture records the stencil state. A million-vertex polygon draws and
  picks (`ternary_render.rs`); the Ternary scene gains a filled phase field
  with a hole.
- **Ternary plots** ([#55](https://github.com/shim9610/figgy/issues/55)).
  `ScatterTransform` grows from 144 to 176 bytes with `ternary_lt` and
  `ternary_r`, the triangle's corners in data-area NDC; the pick core's copy
  follows. `common/projection.wgsl` gains `ternary_ndc` and
  `common/sample_validity.wgsl` `ternary_sample_valid` (a negative or
  non-finite component or a sum of zero or less is masked). The scatter and
  line vertex bodies take the placed position, so the Cartesian and polar
  entries are unchanged and five ternary entries (`vs_ternary`,
  `vs_ternary_mapped`, `vs_pick_ring_ternary`,
  `vs_pick_ring_ternary_mapped` and the line's `vs_ternary`) read c from one
  or two more vertex slots, with pipelines compiled only for a ternary
  chart. The arc scan reads c through `ArcParams.c_base`, and its cache key
  names the c column. The pick variant `PICK_VARIANT_TERNARY` (512) places
  markers, strokes and dashes with the draw's projection, reading c through
  the size map's unused lanes, and a ternary series always takes the direct
  scan. The grid, edges, ticks, labels and titles are emitted through the
  sinks (`axis_render/ternary_axes.rs`), so drawing, hit regions, the
  selection outline and the vector exports share one placement. Fits leave
  a ternary chart's axes alone, annotations place ratio anchors only, and
  streamed sources are rejected. The playground's Ternary scene joins the
  decoration oracle's catalogue. Guide: `TERNARY.md`.
- **Range bars** ([#54](https://github.com/shim9610/figgy/issues/54),
  [#52](https://github.com/shim9610/figgy/issues/52)). The bar module draws
  them through its range bar fragment (`common/range_bars.wgsl`,
  SHADER_COMMON.md §5.18): one instance per row, its whisker, body and
  connector built from the pool on every draw, pick and highlight over the
  category bar group 3, with the part colours in the series' bar style
  slots. `CategoryBarParamsGpu` grows from 48 to 64 bytes with `slot_half`,
  the half slot the category bars and boxes now read too (one category,
  bit-identical). The body, the whisker and the connector pick their bar as
  a histogram bin (`pick_range_bar`); value labels place at the body's end
  (`VALUE_LABEL_KIND_RANGE`). `Chart::auto_fit_layer_union` fits the value
  axis to the ends and whiskers without forcing zero, and a numeric
  position axis to the slots (`range_bar_extents`). Streamed sources are
  rejected. Three playground scenes (Waterfall, Candlesticks, Gantt) join
  the decoration oracle's catalogue. Guide: `RANGEBARS.md`.
- **Offset spectra** ([#50](https://github.com/shim9610/figgy/issues/50),
  #58 item 22a). A document with `line_offsets` draws each member as an
  internal panel of its own through the existing per-panel path: lines,
  fills, markers, error bars, dashes, the picked ring and the PNG, SVG and
  PDF exports follow the member's panel, with no shader change and no
  transform, binding or cache of its own. The view counts member panels
  like overlays (`overlay_count()`, `layer_raster_generations()`, the
  refresh rule of `layer_views`), and a member draws no backdrop and
  reports no tick warnings. A pick on the owner layer answers with what is
  painted under the cursor (the approved D7 policy): the point pick tests
  each member at its own panel's transform and reads back each member's
  candidate beside the final one; the last member with a hit at distance 0
  answers first, else the nearest hit of any series; a typed pick lets a
  member's point hit painted under the cursor answer before the data under
  it.
  Reported indices and values are the pool's, unshifted.
  `Chart::auto_fit_layer_union` fits every member's extents plus its shift
  (`Chart::line_offset_fit_extents`). A nonresident (streamed) member is
  rejected with `FiggyError::InvalidSeriesConfig`, and the stream request
  paths reject a document with members (`InvalidConfig { field:
  "line_offsets" }`). The registry classifies `line_offsets` as a raster and
  geometry edit. Migration: `GpuDataPickTicket` and the internal pick entry
  points changed only inside the crate; the public API adds
  `Chart::line_offset_fit_extents`.

- **Math layout from the OpenType MATH table**
  ([#45](https://github.com/shim9610/figgy/issues/45), #58 item 19). A
  `Math` node in any rich text (titles, axis titles, legend labels,
  annotation and colourbar texts, series labels) is laid out by
  `text_render/math_layout.rs` from the MATH table of the math face:
  scripts, fractions and binomials, radicals with an index, big operators
  (the display variant in display style, limits by `Limits`), operator
  names, stretchy `\left`/`\middle`/`\right` and fixed `\big`…`\Bigg`
  delimiters (size variants, then glyph assemblies), fixed and wide accents,
  and over/under rules, braces, arrows and labels, with TeX's inter-atom
  spacing. A formula starts in its root's style: text by default, or
  display (`$$…$$`), where `\sum` takes its display variant and its limits
  above and below and fractions their display gaps; script styles use the face's
  `ssty` alternates. Measurement and drawing are one placement through an
  internal glyph sink, so a formula's measured extents (and with them its
  bounds, hit test, selection box, drag and title band) are its ink.
  Math glyphs come from the math face in every draw style; `\text{}` keeps
  the text chain (Comic Neue in sketch mode). A cell whose tree does not
  validate draws flat. `CharSet::Math` characters outside math draw from
  the math face too. A `RichText.font` that resolves to a face with a MATH
  table is used as the math face. `validate_renderer_config` refuses a
  malformed tree in a title, axis title or legend
  (`FiggyError::InvalidConfig`), and series registration one in a label
  (`InvalidSeriesConfig`).
- **Bundled math font.** New Computer Modern Math
  (`fonts/NewCMMath-Regular.otf`, 1,306,268 bytes, about 839 KB gzip in the
  wasm bundle) ships unmodified under the GUST Font License, its text in
  `fonts/LICENSE-NewCMMath.txt` with the LPPL it refers to. Other math faces
  can be registered with `register_font_bytes` and named in
  `RichText.font`.
- **New dependency.** `read-fonts 0.46` (`default-features = false`,
  feature `std`) reads the MATH table and the GSUB `ssty` alternates. swash
  and egui pin read-fonts 0.41, which has no MATH table, so the lockfile
  holds both versions until they move, when the two are unified
  ([#160](https://github.com/shim9610/figgy/issues/160)). The reader first
  used `ttf-parser 0.25`, which RustSec marks unmaintained
  (RUSTSEC-2026-0192); the renderer no longer depends on it.
- **Power tick labels** write the exponent as a text-mode `Scripts` node
  (D3 of #45) instead of superscript segments. It lays out through the same
  `script_placement`, so the labels' pixels, bounds and parts are unchanged.
- **One cell placement.** `measure_rich_text` and `draw_rich_text` share
  one internal placement of a cell's glyphs, rules, dashes, bands and
  underlines, and the glyph caches are keyed by glyph id instead of
  character. Every pixel and extent is unchanged.

- **Error bars and line fills pick where they are drawn** (G10 of
  [#37](https://github.com/shim9610/figgy/issues/37),
  [#46](https://github.com/shim9610/figgy/issues/46) item 6). `pick_chart`
  (`pick_point` in the WASM wrapper) now hits error-bar stems and caps on the
  quads the error-bar shader draws — per-point widths of the precise style
  map, the hashed corner wobble under Sketch, a Milkyway jet's beam and knot
  envelope; Constellation draws no error bars — and a line's area fill where
  the cursor lies inside it (distance zero, baseline or second series, every
  step mode, polar). An error-bar hit names the bar's point and a fill hit
  the filled segment's nearer endpoint by the line rule, so `PickedPoint` is
  unchanged. On a tie a marker wins over its line, the line over its error
  bars and every stroke or marker over a fill; among fills the later series
  wins. A zero-alpha bar or fill picks nothing. The draw and the pick build
  the geometry from the same new fragments (`common/errorbar_segment.wgsl`,
  `common/area_piece.wgsl`); the draw is pixel-identical. Zero-size markers
  already picked; a test now pins it. Streamed picks are unchanged (streaming
  is not extended). G10 is closed except for band fills: a field drawn with
  `FillMode::Bands` keeps deciding whether a cell paints by its opacity
  alone, so a transparent ramp stop still picks. Its band position needs the
  contour lookup metadata (binding 6), which the field pick cannot bind
  within the storage-binding limit; moving it into the tail of `stops`
  (binding 3) would change the frozen streaming field builders, so that
  option was not taken. The point picker's
  device-limit check now asks for the 7 storage buffers and 3 bind groups its
  exact pass binds (it asked for 6 and 2). No public API or serialized field
  changes.
- **Selection and placement, renderer side**
  ([#38](https://github.com/shim9610/figgy/issues/38); the model side is
  under [Model](#unreleased-model)). Migration:
  - `Renderer::refresh_axis_with_selection` takes the selected element's id
    (`Selectable::element_id`, `Option<&str>`) instead of boxes;
    `Renderer::selection_box(chart, id)` returns the box it draws.
  - A host element registered in its own `HitMap` (its default `element_id`
    is `"custom"`) or a map built for another layer is not highlighted by
    `refresh_axis_with_selection`, which resolves ids on the active layer's
    standard registration; a host draws such a highlight itself from the
    element's `SelectionGeometry::selection_box` with
    `axis_render::draw_selection_box(canvas, &box, origin, scale)`.
    `Renderer::selection_box` and a refresh with a selected id build the
    active layer's `HitMap` once per refresh to resolve the id (a `Vec` of
    boxed elements and a `String` per compared id).
  - Removed: `pick_nearest_point`, `PointPickOptions`, `PointColumnLookup`
    (hosts pick through `pick_chart` / `pick_chart_data`; the CPU twins are
    test support under the `cpu-twin` feature) and `errorbar_extent` (an
    error-bar fit extent comes from the resident columns on the GPU:
    `Renderer::ensure_errorbar_extent_engine`, then
    `begin_errorbar_extent(value_id, lower_id, upper_id)`).
  - `data_render::draw_chart_panel_with_bodies` moved to the compositor
    (`renderer::composite`), which alone orders the slots below the data, the
    data and above the data; `draw_chart_panel_columnar` is unchanged.
- **Shader assembly.** Every WGSL module is spliced from its fragment files
  at compile time by one macro, `wgsl_module!` (`shader_assembly.rs`), so a
  block several shaders share lives in one file
  ([#46](https://github.com/shim9610/figgy/issues/46)). No shader changes
  what it computes. Migration: `data_render::vector::assemble` returns
  `&'static str` (the module is a constant now, also exported as
  `VECTOR_POINTS_SHADER` / `VECTOR_GRID_SHADER`, each a
  `shader_assembly::Assembled` whose `source` is the WGSL and whose
  `fragments` are the files it is spliced from); call `.to_string()` where
  a `String` was used. `data_render::vector::fragments(input)` is gone: read
  `VECTOR_POINTS_SHADER.fragments` / `VECTOR_GRID_SHADER.fragments` for the
  file list (the fragment contents are no longer exported one by one).
- **The picker runs the draw's projection.** The shared projection, sample
  validity and step fragments take the transform as a parameter, so the GPU
  picker splices the same `projection.wgsl`, `sample_validity.wgsl`,
  `ramp.wgsl` and `line_step.wgsl` the draw shaders run instead of carrying
  renamed copies; the bounded charts' sRGB `output` is one fragment too
  (`srgb_output.wgsl`), and no shader carries a copy of a shared function
  any more ([#46](https://github.com/shim9610/figgy/issues/46)). Each
  module's source and fragment list come from one literal list
  (`shader_assembly::Assembled`), and a test pins every pick module to the
  shared fragments of the draw module it measures. Picks, pixels and the
  pipeline layouts are unchanged.
- **Pick slot keys.** A series' pick resources are keyed by the pick
  descriptor itself, compared field by field, instead of by separate
  signatures; pick style rows carry no colour, so a colour-only edit reuses
  the series' pick slot. The scatter selection ring takes the draw's own
  base radius in both of its branches
  ([#46](https://github.com/shim9610/figgy/issues/46)). No public API,
  serialized field or pixel changes.
- **Histogram selection outline.** A selected bin's outline resolves the
  bin's gap and width ratio with the bar draw's own resolver on the GPU
  (`resolve_bar_style` in `bar_columnar.wgsl`) instead of a CPU copy of it
  ([#46](https://github.com/shim9610/figgy/issues/46) item 2). The selection
  writes the series' bar `Style` with the highlight colour, the outline width
  and the bin's global index in lanes a bar never reads, and a series drawn
  through its bar style map is outlined by the new `vs_bar_selection_mapped`
  entry over the same map. The outline covers the same pixels as before; one
  bind group per selected bin is gone. Migration:
  `data_render::ColumnBarSelectionLayer` drops `selection_bg` and
  `selection_charge` and gains `style_map_bg: Option<&wgpu::BindGroup>`, the
  series' bar style map exactly when the bar draw binds it.
- **Contour labels.** The label atlas code lives in `data_render/contour.rs`
  beside the contour draw ([#58](https://github.com/shim9610/figgy/issues/58)
  item 14). The label quad and the contour stroke gap under it take their
  frame (centre, reading direction, upright turn) from one WGSL fragment,
  `common/contour_label_box.wgsl`
  ([#46](https://github.com/shim9610/figgy/issues/46) item 4). The gap keeps
  its 0.5 px edge inside the quad's 2 px edge; no pixel changes. Migration:
  the public module path `data_render::contour_label_atlas`, which exported
  nothing, no longer exists; remove a `use` of it.
- **Radial inside-label lift.** The vertical offset of a lifted sector's
  inside label is computed once where the sector is built and passed to the
  shader in `Sector.effects.w` (reserved zero before)
  ([#46](https://github.com/shim9610/figgy/issues/46) item 5); the layout,
  the hit map and the shader read the same value. No pixel changes.
- **Script layout.** Superscript and subscript segments, including the
  exponents of `LabelFormat::Power` tick labels, are sized and shifted by
  one renderer function that the math `Scripts` node of #45 will share.
  No pixel, metric or public API changes.
- **Cache keys.** Two renderer caches compared less than their computation
  read, found by the new per-field mutation tests
  ([#46](https://github.com/shim9610/figgy/issues/46)):
  - the resident bodies of a panel were kept after an edit of the tick-label
    face (`bottom_x.label_style.label_font`), which the contour labels baked
    into those bodies use;
  - a fit commit token (`begin_fit_commit` / `commit_auto_fit_all_if_current`)
    stayed valid across an edit of a `Data`-scaled vector's `factor`, a
    histogram's `baseline` or `orientation`, the series membership of an
    overlay or subplot, a subplot's links or an overlay or subplot axis range,
    so a fit computed before such an edit could be committed on top of it.

  The third, in the browser product, is under
  [Browser product](#unreleased-web). No public API or serialized field
  changes.
- **Annotation revision.** `ChartRenderStamp::needs_annotation_raster_since`
  now reports an annotation raster after a demo load whose items differ, a
  column removal whose replacement config edits the items, and a font
  registration (annotation text is drawn with the fonts). The frame was
  already right in all three cases because the raster revision moved; only
  the annotation revision stayed behind.
- **Source layout.** `renderer.rs`, `data_render/mod.rs`,
  `renderer/prepare.rs`, `axis_render.rs`, `gpu_pick.rs` and
  `column_pool.rs` are split into one file per concern
  ([#46](https://github.com/shim9610/figgy/issues/46)). Moves only: every
  public path resolves as before, and no item changes its signature or
  behaviour.
- **Decorations draw above data on every layer**
  ([#47](https://github.com/shim9610/figgy/issues/47)). A layered document
  drew each panel whole, grid, data and decoration, before the next, so an
  overlay panel's data covered the base panel's axes, tick labels, titles
  and legend (the pressure markers of a twin-axes chart over its left axis)
  and a subplot overlay's data its subplot's, while a single-layer chart
  draws its decoration over its data. The compositor now draws the grid and
  data of every panel in layer order, then the decoration of every panel in
  layer order (`composite::FRAME_SLOTS`), on screen (`paint`,
  `paint_prepared`) and in `export_document_*` alike; the selection boxes and
  the annotation overlay stay above every decoration. **Behaviour change:**
  overlay and subplot-overlay data no longer covers the base panel's axes and
  labels. A single-layer chart draws the same pixels, and so does
  `draw_chart_panel_columnar`. Several charts passed to one `paint` call are
  ordered the same way: every chart's data before any chart's decoration,
  which changes pixels only where the charts overlap. No API or serialized
  field changes.

- **The bounded group, classified per chart**
  ([#40](https://github.com/shim9610/figgy/issues/40) item 13). The radial
  chart keeps its own renderer, on the shared sinks, title and hit map, with
  a GPU pick; category bars and box plots draw through the bar module and
  radar series through the polar line, area and scatter paths; the three
  legacy renderers and the bounded chrome are removed. Migrations, the look
  changes and the oracle exclusions are under
  [Migrating from the bounded group](#unreleased-bounded-migration).
  - **Radial decorations on the shared sinks and hit map**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13b). The radial
    frame's title, plot area and slice labels are `Selectable<RadialChart>`
    elements of a `HitMap<RadialChart>`: each places its parts from the chart
    through one emit into the `model::sink` sinks, and the label raster draws
    with the same emit, so hit regions, selection boxes, drag containment and
    the drawn highlight come from one placement. An outside label's leader line
    is a part of the label: it starts at the slice, follows the label's offset
    from its elbow, and is hit and outlined with the text part by part (the
    oracle's leader-line exclusion is gone). **Behaviour change:** the selected
    outside label's highlight outlines each text line and the leader line
    instead of one box around the text; a drag is contained on the text it
    moves. `RadialFrame::decorations()` returns the `HitMap<RadialChart>`
    instead of a `DecorationMap`; query it with the chart the frame was prepared
    from. `decoration_at`, `decoration`, `decoration_id`, `decoration_ids`,
    `decoration_bounds`, `decoration_selection_box`, `drag_decoration`,
    `decoration_resize_handle` and `resize_decoration` wrap the trait calls as
    the `DecorationMap` methods did, and
    `BoundedAnnotationFrame::route_active_layer` routes the annotation layer
    over them.
  - **Radial title band.** The radial title is drawn through
    `emit_chart_title_in`, centred in the chart's own title band across the
    canvas. **Behaviour change:** a default title moves about 2 px down (it
    was placed on a fixed baseline); the plot area does not move. A title wider
    than the canvas is drawn clipped instead of refusing the frame with "title
    does not fit"; its hit region, selection box and drag containment are its
    drawn extent intersected with the canvas.
  - **Radial labels draw their SSOT text.** The radial layout draws
    `RadialSlice::text` and formats no value. **Behaviour change:** `prepare`
    refuses (`RadialError::Invalid`, naming `format_labels`) a chart where a
    slice with a value shows a label without text. The outside-label stacking
    keeps its order and arithmetic in fixed arrays of `MAX_RADIAL_SLICES`.
  - **Radial slices are picked on the GPU.** The sector layout and its ray-march
    are the shader fragment `data_render/common/radial_sector.wgsl`, spliced
    into the draw module and a new pick module (`radial_pick`); the pick runs
    the draw's `hit_pixel` and occlusion rule over the frame's own sector
    buffer. `RadialFrame::hit_test` and `hit_test_at_rest`, whose CPU twin was
    product code, are replaced by `RadialFrame::pick(position)` and
    `pick_at_rest(position)`, which submit the pick and return a
    `RadialPickTicket`; `resolve().await` (natively `resolve_blocking()`) reads
    the slice back, like the Cartesian pick ticket. The frame no longer keeps
    its sectors on the CPU; the CPU twin is test support under the `cpu-twin`
    feature.
  - **Radial frame key.** The frame cache is keyed by `FrameKey`, the chart's
    fields that reach the frame named one by one, instead of a clone of the
    chart; a mutation test classifies every field
    ([#66](https://github.com/shim9610/figgy/issues/66)). Editing
    `label_format`, `label_decimals` or `value_suffix` without writing the
    labels again reuses the frame, since nothing it draws changed.
  - **Categorical axes** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13c). The four Cartesian axes and the overlay Y axes may be
    categorical (`AxisOptions::categories`). The category labels are the
    side's `AxisLabel` element, placed by the numeric labels' placement in the
    axis' label style, so hover, hit, selection, both drag offsets, the picked
    highlight and the PNG export are the existing ones; the data transform,
    the annotation space and the view state map the category range. The
    renderer gate applies `validate_categories` and rejects a categorical
    colourbar axis with `InvalidConfig`; a streamed series on a chart with a
    categorical axis is rejected with `InvalidSeriesConfig`. The range
    helpers (`set_*_range`, `auto_fit_*`, view states) leave a categorical
    axis unchanged. With labels "0", "1", … a categorical axis draws pixel
    for pixel what the numeric axis over the same range with a tick at every
    integer draws.
  - **Axis walks on the plot rectangle.** The Cartesian axis, tick, tick
    label and grid walks take the plot `RectF` instead of the integer data
    area. Every existing document draws the same pixels.
  - **Category bars on the bar path**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13d). A
    `CategoryBars` series draws through the bar module: the input fragment
    `data_render/common/category_bars.wgsl` (SHADER_COMMON §5.13) is spliced
    into `bar.rs::BAR_SHADER` beside the histogram entries, one instance per
    category, the rect built on every draw from the positions, the group
    layout and, for a stack, the other series' values read by their pool
    lanes from the series' lane table (group 3, bindings 8 to 10: the pool,
    `CategoryBarParamsGpu` and the lane table, #107 R3). The bars are shaded by `bar_surface` with their
    material; the histogram's override table carries the per-bar overrides
    (`BarStyleSlotGpu` / `BarStyleOverrideGpu` gain the `material` and
    `finish` lanes, `BAR_STYLE_MASK_MATERIAL`). The typed data pick
    (`pick_category_bar`, answered as `PickedData::HistogramBin` with the
    category row), the selection outline and emphasis, `step_selection`,
    hover, PNG export, the resident body cache (its key gains `bar_groups`
    and the category axes) and the overlay and subplot panels are the
    histogram's. The value-axis fit (`Chart::auto_fit_layer_union`,
    `category_bar_value_extent`) covers zero and each column's range for
    grouped bars, `0..100` for percent bars, and for stacked bars
    `[Σ min(0, min_i), Σ max(0, max_i)]` over the value columns' own
    statistics, an upper bound when the series peak in different
    categories. A `PercentStacked` group whose value column's minimum
    statistic is below 0 is refused when drawn or picked, with
    `InvalidSeriesConfig` naming the series and the mode, since the shader
    stacks only positive shares. Value labels are drawn from `labels.text` (#40
    item 13h, below).
    `bar_groups.bar_gap` is logical pixels, scaled with the display config
    (`Config::scaled`), and `group_width` a share of the slot, so a 2x export
    is the 1x layout doubled (`category_bars_render` and `box_plot_render`
    check it).
    A streamed category bar series is rejected with `InvalidSeriesConfig`.
  - **The legacy categorical renderer is removed.** `CategoricalRenderer`,
    `CategoricalFrame` (`bar_rect`, `hit_test`, `decorations`, …),
    `CategoricalError`, `categorical.wgsl` and `categorical_layout.rs` are gone;
    `renderer::categorical` is the model's document module.
  - **Axis crossing.** `AxisOptions::cross_at` moves the axis line and its hit
    area to the value of the perpendicular axis (the ticks, labels and title
    stay at the plot edge)
    (`layout::axis_cross`); a non-finite value is rejected with
    `InvalidConfig`. `None` draws every existing document the same.
  - **Look of a categorical document.** Opened through the reader, a
    document draws with the Cartesian chrome instead of the former bounded
    layout: the data area follows the Cartesian margins (the explicit
    `decorations.data_area` is not carried), the legend is the Cartesian
    legend box in its corner instead of the row below the chart (the bottom
    row legend is gone), the export clear colour replaces the document's
    `background`, and value labels are drawn through the contour label
    module (#40 item 13h, below; the changed-pixel counts here were measured
    before they were). The bars themselves shade as before: the dominant bar
    colours of the former renderer's and the new exports of the same
    documents are equal for Flat, Enamel and Paper and within 1/255 for
    SatinMetal and 2/255 for Matte, whose grain pattern differs. Changed pixels
    of whole 900 × 620 exports of the five `categorical_gallery` cases
    against the former renderer: grouped-matte 533 431 (95.6 %),
    horizontal-satin 514 021 (92.1 %), stacked-enamel 369 997 (66.3 %),
    percent-outline 373 719 (67.0 %), signed-stacked 454 125 (81.4 %); the
    reader inverts the category axis of a horizontal document, so its first
    category is at the top as before (the two horizontal cases drew it at the
    bottom before that, 557 936 and 386 412 changed pixels); the
    released 0.12.1 edited document at 800 × 600: 446 660 at 1x, 1 788 971
    at 2x. None is pixel identical; every difference is the chrome above.
  - **Record and ragged column sources in the pool**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13e). Two
    `ColumnSource` extensions for the box plot, with no drawing consumer yet:
    a record (`RecordSource { schema, fields }`, a fixed set of named scalar
    fields per row checked by name against a static `RecordSchema` such as
    `BOX_PLOT_RECORD`, at most `MAX_RECORD_FIELDS` = 12) and a ragged column
    (`RaggedSource { values, offsets }`, group `g` being
    `values[offsets[g]..offsets[g + 1]]`, its offsets stored as `u32` index
    lanes). `ColumnPool::upsert_record` / `upsert_ragged` upload the members
    as one batch transaction (one region, one staging buffer) and measure
    `RecordStats` (per field: presence, finite range, finite and NaN counts)
    and `RaggedStats` (the values' range and counts, the offsets'
    `non_decreasing` flag, the longest group) in the write observer; a
    replacement takes a new epoch and frees the old members, and
    `remove_record` / `remove_ragged` delete them. `record(id)` / `ragged(id)`
    return `RecordRef` / `RaggedRef` (member handles), `record_stats` /
    `ragged_stats` the statistics. A schema mismatch, unequal field lengths,
    a non-monotone offsets column and the other rejections are
    `FiggyError::InvalidSeriesConfig` naming the id, with nothing published.
    `Renderer::upsert_record`, `upsert_ragged`, `remove_record` and
    `remove_ragged` (forwarded by `WindowedRenderer`) wrap them with the picker
    and invalidation preparation of `add_columns`. Records and ragged columns are resident only: upserting onto
    a streamed id, or registering a stream onto a record or ragged id, is an
    `InvalidSeriesConfig` that names the limitation. Columns, records and
    ragged columns share one id space; a member column (internal id) cannot be
    inserted, replaced or removed as a column (`ReservedColumnId`) and a series
    cannot reference it. No serialized field and no shader change; the
    additions are API only.
  - **Box plots on the bar path**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13f). A
    `BoxPlot` series draws through the bar module: the fragment
    `data_render/common/box_plot.wgsl` (SHADER_COMMON §2.5 and §5.15) is
    spliced into `bar.rs::BAR_SHADER` with the marker, error bar segment,
    ramp and sketch noise fragments, one instance per category with
    `BOX_PLOT_BASE_VERTICES` (42) vertices plus six per outlier of the longest
    group (`RaggedStats::max_group`), reading the record's fields and the
    outliers by their pool lanes through the category bar binding layout
    unchanged (group 3, bindings 8 and 9: `CategoryBarParamsGpu`, whose lanes
    a box plot reads as its record, outlier and group lanes). The body is
    shaded by `bar_surface` with its material and drawn with the former
    renderer's notch polygon; the whiskers, caps and median are
    `ErrorBarSegment` quads and the mean and outliers `marker.wgsl` shapes,
    all built on every draw. The parts' styles are three slots of the bar
    style map (`PrimitiveStyle::from_box_plot`,
    `create_bar_style_map_with_slots`, the `BAR_STYLE_MASK_FILL`, `_BORDER_*`,
    `_GAP` and `_WIDTH_RATIO` masks) and the per-box overrides its override
    rows (`row * 3 + part`, a fixed array, no new binding).
    `ColumnCategoryBarLayer` gains `vertices`. The typed data pick
    (`pick_box_plot`, answered as `PickedData::HistogramBin` with the
    category row) tests the body, the whisker and cap and median strokes, the
    mean and every outlier of the box, each answering its box; the selection
    outline (an inside ring around the drawn body, the notch included) and
    emphasis, `step_selection` (the boxes, not the outliers), hover, PNG
    export, the resident body cache (its key gains the box plot category
    axes, and its column fingerprint the record and ragged member columns)
    and the overlay and subplot panels are the category bars'. The
    value-axis fit (`Chart::auto_fit_layer_union`, `box_plot_value_extent`)
    covers the pool's statistics of every value field of the records (all
    but `sample_count`) and of the outlier columns, hidden means and outliers
    included as before. Nothing per box is stored on the CPU.
    **Unordered statistics are clamped, not rejected**: the renderer cannot
    read a record on the CPU, so it no longer checks the order the former
    renderer checked. The body spans `min(q1, q3)..max(q1, q3)`, the median
    is clamped into it, the stems run from `min(whisker_low, body low)` and
    to `max(whisker_high, body high)`, a notch needs both CI endpoints, and a
    row with a missing required field draws nothing. A `BoxPlotChart` still
    validates the order, so a document drawn through the reader draws as
    before. A streamed box plot series (or a streamed record or outlier id)
    is rejected with `InvalidSeriesConfig`, and a polar chart rejects box
    plots.
  - **The legacy box plot renderer is removed.** `BoxPlotRenderer`,
    `BoxPlotFrame` (`hit_test`, `part_rect`, `box_rect`, `plot_rect`,
    `decorations`, …), `BoxPlotPick`, `BoxPlotPart`, `BoxPlotError`,
    `boxplot.wgsl` and `boxplot_layout.rs` are gone; `renderer::boxplot` is the
    model's document module. The `boxplot_editor` example is removed;
    `boxplot_gallery` draws through the reader.
  - **Look of a box plot document.** Opened through the reader, a document
    draws with the Cartesian chrome instead of the former bounded layout:
    the data area follows the Cartesian margins (the explicit
    `decorations.data_area` is not carried), the legend is the Cartesian
    legend box in its corner, the export clear colour (transparent in the
    gallery) replaces the document's `background`, a selected box draws the
    Cartesian inside outline, an unfilled outlier is the marker's 1.5 px ring
    (formerly at most 1.2 logical px inside the marker), and value labels are
    drawn through the contour label module (#40 item 13h, below; the
    changed-pixel counts here were measured before they were). The boxes
    themselves shade as before: the dominant box
    colours of the former renderer's and the new exports of the same
    documents are equal. Changed pixels of whole 960 × 640 exports of the
    five `boxplot_gallery` cases against the former renderer, composited
    over white (raw RGBA in brackets, the clear colour differing): classic
    matte 106 874 (17.4 %) [600 005], satin 93 480 (15.2 %) [587 033],
    horizontal 98 512 (16.0 %) [600 129], grouped notch 92 956 (15.1 %)
    [600 227], mean and selection 95 304 (15.5 %) [600 168]; at 2x 385 432,
    340 208, 373 575, 328 413 and 335 510. A horizontal document with
    per-box overrides: 92 608 (15.1 %) at 1x, 344 896 at 2x. The released
    0.12.1 edited document at 800 × 600, whose background is not white:
    479 241 (99.8 %) raw, 90 046 with a channel difference above 8; at 2x
    1 916 942 and 352 881. None is pixel identical; every difference is the
    chrome above.
  - **Categorical polar angle axis**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13g). The polar
    projection reads its angle constants from `AngleMap`, so a categorical
    angle axis places category `i` at `i / n` of the turn or sector with no
    shader change; a numeric axis draws pixel-identically. The polar axes
    draw a categorical axis' spokes at the categories with
    `categories[i].label` as spoke labels (an empty label draws nothing and
    registers nothing), the rings at the radius majors and no minor angle
    ticks, and a `Polygon` grid as chords between the category spokes. Hover,
    click selection, decoration drag (the ring labels along the ray, the
    spoke labels along their spokes, the ray turning), the plot area, the
    annotation layer, subplots and PNG export follow the polar rules
    unchanged.
  - **Radar series on the line and scatter paths**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13g). A `Radar`
    series draws as the polar line layer with a new `closed` flag, one more
    instance that joins the last vertex to the first, its fill to the inner
    ring through the area layer (`closed` too, on a full circle; a sector
    leaves the closing wedge unfilled) and its vertex markers through the
    scatter path, all built on every draw from the two columns; nothing per
    vertex is stored on the CPU. The GPU pick gains the closing segment
    (`line.z` of the pick query, `pick_next_point` and `pick_starts_segment`
    in the pick core, a closed line scanned directly) and the closed fill,
    with the tie rule markers over the outline over the fill, and answers
    `PickedData::Point` with the category row as `point_index`; a picked
    vertex draws the picked ring. The radius-axis fit
    (`auto_fit_layer_union`) covers the value columns. A radar series is
    drawn in the precise draw style only; a radar series on a rectangular
    chart, a numeric angle axis, an `Arc` interpolation, more than 16 radar
    series in a layer or columns that do not hold one value per category are
    rejected with `InvalidSeriesConfig`, and a streamed radar series with
    "radar series require GPU-resident columns; streamed sources are not
    supported".
  - **The legacy radar renderer is removed.** `RadarRenderer`, `RadarFrame`
    (`hit_test`, `part_rect`, `vertex_position`, `plot_rect`, `plot_circle`,
    `decorations`, …), `RadarPick`, `RadarPart`, `RadarError`, `radar.wgsl`,
    `radar_layout.rs` and its `polygon_distance` are gone, with the bounded
    decoration helpers only they used; `renderer::radar` is the model's document
    module. The `radar_editor` example is removed; `radar_gallery` draws through
    the reader.
  - **Look of a radar document.** Opened through the reader, a document draws
    with the polar chrome instead of the former bounded layout: the plot is
    the polar circle inside the Cartesian margins (the explicit
    `decorations.data_area` is not carried), the legend is the Cartesian
    legend box, the export clear colour replaces the document's
    `background`, the ring labels are the radius tick labels on the ray, a
    picked vertex draws the picked ring instead of the brightened series,
    the ring labels include the centre's `0`, and value labels are drawn
    through the contour label module (#40 item 13h, below; the changed-pixel
    counts here were measured before they were). The polygons, fills and
    markers themselves keep their places.
    Changed pixels of whole 960 × 640 exports of the five `radar_gallery`
    cases against the former renderer, composited over white (raw RGBA,
    the clear colour differing, is 100 % in each): spider web 67 747
    (11.0 %), circle values 72 870 (11.9 %), fixed axis 100 298 (16.3 %),
    four series 132 665 (21.6 %), selection 111 500 (18.1 %); at 2x
    251 852, 269 546, 384 227, 514 080 and 429 467. The released 0.12.1
    edited document at 800 × 600, composited over its own background:
    94 450 (19.7 %), 92 855 with a channel difference above 8; at 2x
    371 662 and 365 452. None is pixel identical; every difference is the
    chrome above.
  - **Value labels through the contour label module**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13h). Category
    bar, box plot and radar series draw their value labels again: each
    series' `labels.text` strings are baked into one label atlas by the
    contour labels' builder (`bake_contour_label_atlas`: the one text stack,
    `FontPolicy`), in the value axis' tick label face and size at the render
    scale and its colour (the radius axis' for a radar series; white inside
    a dark bar), cached per series in the contour labels' scratch cache
    (its entry is now `ScratchEntry::Contour` or `ScratchEntry::ValueLabels`,
    one map with one eviction rule) under the atlas input (the strings, the
    face, the size, the colour) and the font generation, and
    placed on every draw from the pool by the bar module's value label
    entries: at the bar end (outside, inside, or `Auto`: outside grouped
    bars, inside stack segments that fit), past the top of the box (its high
    whisker, mean, notch and outliers), or outside the radar vertex along its
    spoke, as the former charts placed them, moved by `labels.offsets` and
    sampled texel for texel (an upright label's quad reaches the atlas
    cell's gutter, so it needs no clamp; the contour labels keep theirs).
    A radar series' `Emphasized` labels draw while it has a selected vertex
    or label. The contour label's quad placement is the shared
    fragment `common/label_quad.wgsl`; the value labels are
    `common/value_label.wgsl` in the bar module, after the category bar and
    box plot input fragments. An empty string draws nothing; labels never
    displace each other (the former `LabelPlacer` is gone: thinning the
    category axis is the only thinning). Labels are drawn after every
    series' picked highlights of their panel. A label that would cross the
    data area's edge past its anchor goes to the other side of it (inside
    the bar's end, below the box's top, inward along the radar spoke), in
    the shader for the draw and the pick alike; what still overflows is
    clipped to the data area.
  - A label is a typed pick target: `pick_chart_data` answers
    `PickedData::ValueLabel { source_id, series_id, category_index,
    distance_px }` within the pick distance of its selection box, with a
    paint order after every series of the panel, so it wins every tie
    against the data under it (`pick_value_labels`, candidate kind 4). A
    selected label (`PickedDataRef::ValueLabel`) draws its selection box
    around its frame, the decorations' selection policy (`SELECTION_PADDING`,
    `SELECTION_STROKE_WIDTH`, `SELECTION_COLOR`). PNG export bakes the atlas
    at the export scale; labels of overlay and subplot series draw and pick
    in their panels.
  - **Bounded leftovers removed** (13i). `decoration::DecorationMap`, which
    had no constructor left after the radar renderer, and
    `BoundedAnnotationFrame::hit_test_active_layer`, which took one, are
    removed; the radial frame answers through `HitMap<RadialChart>` and
    `route_active_layer`. So is the annotation underlay path no chart reached:
    `FrameBackground` (`Opaque`, `HostClear`), `RadialRenderer::set_background`
    and `background`, `RadialFrame::background`,
    `BoundedAnnotationFrame::draw_underlay` and `draws_underlay`, and the
    radial export's host-clear branch. The radial chart has no value axes, so
    a span on it was always skipped with a warning; it draws its frame with
    its opaque background and the annotations above it, as before, and its
    frame cache holds one frame instead of one per background mode. The
    annotation admission counts no underlay texture for it. The `decoration`
    module keeps the interaction guide; `BoundedAnnotationFrame` keeps its
    name. The `bounded_titles` example is `radial_titles`. No shader or GPU
    struct changes; the radial pixels and picks are unchanged.
- **Adaptive log-axis labels**
  ([#42](https://github.com/shim9610/figgy/issues/42),
  [#58](https://github.com/shim9610/figgy/issues/58) item 20). On an axis
  with `log_ticks: Adaptive` the tick-label pruning first demotes crowded
  labels to the next sparser ladder, 1..9 → 1, 2, 5 → decades, while at
  least two labels remain, then thins what is left by the existing 1, 2, 5,
  10… stride ("every n-th decade"). The half-em gap and the box test are
  unchanged, so the labelled set is the same at every output scale; the
  ticks and grid lines are not thinned. The polar ray and the colourbar
  labels go through the same pruning. An adaptive axis raises no
  `RenderWarning::LogDecadeStepAdjusted`, as calendar ticks raise no spacing
  warning. Under the default `Decades` policy every label, pixel and warning
  is unchanged. Streamed charts accept the policy (axis chrome only). The
  playground's chart panel has "adaptive log X" and "adaptive log Y"
  switches, and the `LogAxes` catalogue scene draws its energy axis, which
  spans under one decade, logarithmic with the adaptive policy (1..9 ladder,
  Decimal labels).

- **SVG and PDF export** ([#48](https://github.com/shim9610/figgy/issues/48),
  [#58](https://github.com/shim9610/figgy/issues/58) item 21). Additive API:
  `Renderer::export_document_svg_async(chart, series, scale) -> String` and
  `export_document_pdf_async(..) -> Vec<u8>`, their `_with_clear_async`
  variants (the clear colour as a background rectangle), the blocking native
  `export_document_svg` / `export_document_pdf`, the same six methods on
  `WindowedRenderer`, and `RadialRenderer::export_svg_async(chart, size,
  scale)` / `export_pdf_async` with blocking `export_svg` / `export_pdf`. The
  decorations are vector content written from the emit the raster draws
  with, at the PNG export's scaled and placed document, so the file's
  decorations are the drawn ones: strokes with the raster's widths, width
  floors, dashes and sketch curves, text as glyph outlines from the mask's
  scaler, hinting and embolden (no font embedded; the text is not
  searchable), the annotation layer, polar chrome. The data layer is drawn by
  the GPU, so each panel group (a chart and its overlays, each subplot) is one
  image of its data areas, the PNG export's own pixels; the colourbar ramp and
  the Milkyway backdrop are images too. The SVG's `viewBox` is in physical
  pixels and its size, like the PDF page, the logical size in points
  (1 px = 0.75 pt); `scale` is clamped as for the PNG, and a streamed source
  is rejected with `ColumnNotResident`. The PDF is written by hand (objects,
  cross-reference table, deflated content, one ExtGState per alpha byte, RGB
  image XObjects with soft masks).
  - **Documented omissions** (D5): the Milkyway glow is raster-only and not
    drawn; no selection box is drawn (as in the Cartesian PNG; the radial PNG
    draws the selected decoration's box, its SVG and PDF do not); the radial
    export draws every inside label above every slice, where the PNG can hide
    one under a lifted neighbour.
  - **New direct dependency**: `flate2 = "1"` (1.1.10, MIT OR Apache-2.0),
    already in the lockfile through `png`; `Cargo.lock` gains only the
    renderer's dependency line.
  - Internals, no pixel change (the 17 catalogue scenes and the radial scene
    export the same RGBA at 1x and 2x): the text placement is
    `place_rich_text` / `place_plain_text` over the glyph sink, the sketch
    curve `sketch::smooth_cubics`, the annotation item loop
    `emit_annotation_layer`, the radial split connectors strokes into the
    label sink (`emit_split_connectors`; the guard's `draw_line` allowance for
    `radial_layout.rs` is removed), and the export `prepare_export` /
    `paint_export`.

- **Broken axes** ([#200](https://github.com/shim9610/figgy/issues/200)).
  The renderer draws `AxisOptions::breaks` in both styles. `Continuous`:
  `ScatterTransform` gains `breaks: [[f32; 4]; 6]` and `break_info: [f32;
  4]` (the uniform grows from 176 to 288 bytes, `PickQueryParamsGpu` from 592 to
  704), and `axis_pair_to_t` remaps data `t` through the cut table
  (`cut_remap`) in every module that places data through it, the pick
  modules included. `Panels`: the transform maps the axis at its first
  piece, and each shifted piece is drawn with a uniform of its own whose
  panel offset is moved by the piece's shift, so a mark crossing a gap
  keeps its own slope in each piece. In both styles the data slot (fills,
  bodies, picked highlights and value labels, on the screen and in the
  resident and export composites) is drawn once per piece, scissored to
  it, so nothing is drawn in a gap; no shader masks a fragment. A pick
  whose cursor is in a gap answers nothing, and under `Panels` the cursor
  and the data-area clip are moved back by the piece's shift. Streamed
  sources on a broken axis are refused with `STREAMED_BREAK_REASON`; the
  cut ranges, style and gap move the view revision
  (`break_geometry_equal`), the mark and frames only the raster.
  The shader pipeline contract is regenerated. Migration: a
  `ScatterTransform { .. }` literal adds `breaks: NO_BREAKS` and
  `break_info: NO_BREAK_INFO` (`data_render::NO_BREAKS`,
  `NO_BREAK_INFO`).

- **Energy-level (Grotrian) diagrams** ([#203](https://github.com/shim9610/figgy/issues/203)).
  An `EnergyLevels` series is drawn on the CPU: at prepare its parts are
  rastered through the canvas sink into one panel-sized texture, in logical
  pixels at the series' display scale, cached in the scratch map by the
  panel config, the series, the records it draws and the font generation,
  and composited in the data slot after every picked highlight, per data
  piece (approved on #203 as an exception to computing appearance in the
  shader). The column-only paths skip the DataLess series. A typed data
  pick answers for it on the CPU over the drawn placement (a level as a
  `HistogramBin`, a transition as a `Point`, a label as a `ValueLabel`
  naming its `part`); `GpuDataPickTicket` merges that hit with the GPU
  hits, and `PickedData::ValueLabel` gains `part`. A subplot nested in its
  parent's data area (an inset) is drawn after every other panel, all its
  slots, and a base-layer pick inside it answers for the inset. A
  streamed display refuses the series (`STREAMED_ENERGY_LEVELS_REASON`).
  The vector export carries it in the data image like any series (#234
  makes it vector content). New
  `nist_grotrian` example: a std-only NIST ASD CSV parser over bundled He
  I exports. Migration: a `PickedData::ValueLabel { .. }` literal adds
  `part: None`; a `SeriesLayers` literal adds `energy_levels: None` (or
  starts from `SeriesLayers::empty()`).

- **Grotrian diagrams at atlas scale** ([#205](https://github.com/shim9610/figgy/issues/205)).
  Each inset is painted as one unit (data, chrome, highlights) in the tree
  order of `layout::inset_paint_order`: after the panel it is nested in and
  before that panel's next sibling, so an inset drawn on top takes the
  pointer. A base-layer pick (`pick_chart`, `pick_chart_data`) whose cursor
  lies in a subplot's frame runs on that subplot (`Config::subplot_at`), as
  if it were the active layer. The shader reads each broken piece's slope
  off its bounding knots (`cut_piece_slope`) for `Equal` and `Weights`
  sizing, with no new uniform, and the vector and line-pick derivatives take
  `cut_scale_at` at the point they measure; staggered category labels,
  per-piece ticks, level bands and the label layout draw through the
  existing axis and energy-level paths. New `Renderer::hold_chart_display(id,
  hold)`: while held, the document keeps the display scale and the position
  of its parent chart it had when the hold began, so a subplot dragged past
  the document's edge does not rescale the page under the pointer; the
  release fits the document to the surface again. The `nist_grotrian_gallery`
  example draws He I, Ar I, Cu I and U I from trimmed NIST exports through
  `support/nist_csv.rs` and `support/grotrian_sets.rs`, shared with the
  playground's `EnergyLevelsAr`, `EnergyLevelsCu`, `EnergyLevelsU` and
  `EnergyLevelsArAll` scenes. The display or export scale reaches the chrome
  emit (the maintainer's option A on #205): new
  `Renderer::create_chart_view_at(chart, panel_rect, scale)` and
  `Renderer::refresh_axis_with_selection_at(view, chart, panel_rect,
  selected, scale)` take the scale a placed or exported chart's pixels
  carry, and the view keeps it for `refresh_axis` /
  `refresh_axis_with_selection`; `create_chart_view` and the plain refresh
  stay at scale 1. `axis_render::try_raster_chart_layer_to_rgba_with_selection_at`
  is the CPU raster at a scale. The export and the vector export pass their
  scale, the web wrapper its display scale, and the playground its display's,
  so a broken axis' piece frames (`frame_each_panel`) are crisp at 1x and
  2x is 1x doubled; the Grotrian scenes draw their pieces framed again.

<a id="unreleased-web"></a>

### Browser product (`figgy` 0.16.0)

- **Themes** ([#265](https://github.com/shim9610/figgy/issues/265)).
  `FiggyChart.apply_theme(name, subplots)` and `<figgy-chart>`'s
  `apply_theme(name, subplots = "all")` apply a built-in theme (`light`,
  `dark`, `paper`, `dashboard` or `sketch`) to the chart, its overlays and
  the series drawn there, then refit the layout; `subplots` is `"all"`,
  `"none"` or a JSON array of indices. `FiggyBoundedChart.apply_theme(name)`
  themes a radial chart. An unknown name or a bad list changes nothing.

- **Math in the text setters** ([#272](https://github.com/shim9610/figgy/issues/272)).
  Breaking. `FiggyChart.set_title`, `set_x_title`, `set_y_title`,
  `set_colorbar_title`, and the labels of `add_line_series` and
  `set_series_label` (and the `<figgy-chart>` methods that forward to
  them) read their text as matplotlib reads a label: `$…$`, `\(…\)` and
  `$$…$$` spans are TeX, `\$` is a dollar sign, and text whose TeX does not
  parse is shown as written. Migration: write `\$` (in a JavaScript string
  literal, `"Price (\\$)"`) where a `$` is meant as a dollar sign. A
  `RichText` JSON value (`set_config`, `add_annotation`, `add_corner_note`,
  `rich_text_from_tex`) is taken as it is.

- **Tick label clearance** ([#272](https://github.com/shim9610/figgy/issues/272)).
  The config JSON's axis `label_style` takes an optional `clearance`,
  `"Auto"` (the default, absent from the JSON) or `"Keep"`, set through
  `get_config` / `set_config` as the label format is. `"Auto"` drops a tick
  label where its box comes within a small visual gap (2 px in total at the
  default 18 px label font, `(a + b) / 18` px between an `a` px and a `b`
  px label) of the label of the axis across at a corner or a break mark's
  slash, so boxes 1 px apart at 18 px count; `"Keep"` keeps every label of that axis.

- **Axis units** ([#273](https://github.com/shim9610/figgy/issues/273)).
  An axis in the config JSON takes an optional `unit`,
  `{ "scale": 0.00015258789, "offset": 0 }`, absent by default, set through
  `get_config` / `set_config` as the label settings are (the colourbar's
  through `set_colorbar_axis` too). The axis then prints
  `scale * value + offset` on round shown steps; the data and the picks
  stay raw. `set_config` refuses a scale that is zero or not finite, an
  offset that is not finite, a log axis' unit with an offset or a negative
  scale, and a unit on a probability, timestamp or categorical axis, each
  with its `code` (`AxisUnitScale`, `AxisUnitOffset`, `AxisUnitLog`,
  `AxisUnitProbability`, `AxisUnitTimestamp`, `AxisUnitCategorical`).

- **A chart background** ([#265](https://github.com/shim9610/figgy/issues/265)).
  The config JSON takes an optional `background` colour that fills the whole
  chart area under everything else. `FiggyChart.set_background(color_json)`
  and `<figgy-chart>`'s `set_background(color)` set it, `null` clears it.

- **Wavelength labels and corner notes** ([#258](https://github.com/shim9610/figgy/issues/258)).
  The series JSON carries `wavelength`, `wavelength_uncertainty` and
  `wavelength_labels` (with its `unit`), and a `Text` annotation an
  optional `align`. `FiggyChart.add_corner_note(text_json, corner)` and
  `FiggyBoundedChart.add_corner_note(text_json, corner)` add a text item
  owning the `RichText` JSON at a `NoteAlign` corner (`"BottomRight"`, …)
  and return its index. It is hit, selected, dragged and hidden like any
  text box.

- **Typed validation reasons** ([#215](https://github.com/shim9610/figgy/issues/215)).
  A refused rule throws a JS `Error` whose `message` is the rule's sentence
  (unchanged) and whose `code` is the rule's name (`RuleKind::code`, for
  example `"ContourLevelCount"` or `"PolarImage"`), so a page branches on
  `error.code` instead of parsing the text. A rule checked while the JSON
  is parsed (a contour label `spacing_px` that is not positive, more than one
  coordinate system, a rich-text segment or math that does not parse) is a
  JSON parse error and, like every other error, still throws its message as
  a string. Migration: code that read a rule refusal with
  `String(error)` or as a plain string reads `error.message`.

- **Feynman diagrams** ([#234](https://github.com/shim9610/figgy/issues/234)).
  `pick_point` answers a Feynman part as `{kind: "feynman_vertex" |
  "feynman_propagator" | "feynman_label", series_id, index, part?}`;
  `press_feynman(series_id, kind, index, part)` selects it and starts the
  drag `on_move` applies: a vertex moves and a propagator bends through the
  panel's axes, and a label moves through the value-label drag, as an
  energy-level label does. The wrapper commits a diagram's layout as its series is
  registered, so `get_series` returns placed vertices; `auto_layout(series_id)`
  commits it again after an edit in place. `selection_details` reports a
  selected vertex (id, role, drawn position, whether it is placed), a
  propagator (id, ends, particle, bend) or a label (part, offset).
- **Triangle index columns** ([#210](https://github.com/shim9610/figgy/issues/210)).
  `register_triangles(id, Uint32Array)` registers or replaces a triangle
  index column, borrowing the array for the upload, and
  `remove_triangles(id)` removes one.
- **Triangle mesh series JSON** ([#210](https://github.com/shim9610/figgy/issues/210)).
  The `TriMesh` render_type and the `mesh_triangle` selection parse and
  write through serde (SCHEMA.md); a `pick_data` hit on a mesh will report
  `kind: "mesh_triangle"` with its `triangle_index` and `vertex_index`.
  `set_series` draws a mesh series on a flat chart.
- **Triangle mesh picks, details and steps** ([#210](https://github.com/shim9610/figgy/issues/210)).
  `pick_data` answers a mesh's triangles (`kind: "mesh_triangle"`, with
  `triangle_index` and `vertex_index`) and its contour strokes.
  `selection_details()` reports a selected triangle's three vertex indices,
  read from the triangle index column's `u32` lanes, and their coordinates
  and values (or its face value), read back from the GPU columns.
  `step_selection` counts a mesh's triangles from the index column's
  statistics. `set_series` accepts a series that names a registered
  triangle index column; it refused every mesh series before, because the
  triangle id is not a column id. On a 3D chart the mesh draws as a surface
  over its `z_column` (the trisurf, #210 step 5) and the details report the
  triangle's heights as `z`, next to its `values`.
- **3D vector fields** ([#211](https://github.com/shim9610/figgy/issues/211)).
  A picked arrow of a 3D point vector reports `z` and `w` in
  `selection_details()` next to `x`, `y`, `u` and `v`. The series JSON
  carries `w` in `components.Points`, omitted when absent.
- **Legend rows follow their series** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `set_series_label`, `remove_series`, `remove_column`, `add_line_series`,
  `set_series` and `load_demo` find a series' legend row by the series its
  symbol names. They no longer use the series' position. Before, a series
  without a label put every later row out of step: relabeling or removing
  a series edited its neighbour's row, and loading the demo again added a
  duplicate row.
- **`set_picked_points` is removed** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The chart has one selection, `Config.picked_data`. Migration:
  `set_picked_points(JSON.stringify({ refs: [{ series_id, point_index }],
  ring_color, ring_width_px, radius_extra_px }))` →
  `set_picked_data(JSON.stringify({ refs: [{ kind: "point", series_id,
  point_index }], highlight_color, outline_width_px, point_radius_extra_px }))`.
- **No second column registry in the wrapper** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The wrapper no longer keeps its own map of column lengths and revisions,
  its revision counter or the pending handoff metadata; registration checks,
  `step_selection` and `gpu_memory_status` read the renderer's registry
  (`logical_column`, `logical_column_ids`, `column_revision`). Behaviour
  change: a resident column's revision in `gpu_memory_status` is now the
  pool's allocation epoch for its latest upload. It still changes on every
  upload, and a streamed column still reports the revision the host declared.
  The JS API is unchanged.
- **No plain-text label mirror** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The wrapper no longer keeps a plain-text copy of every series label beside
  `SeriesConfig::label`. Behaviour change: rebuilding the automatic legend,
  and `set_series` with a series that has no `label`, take the label from the
  series alone. A series whose new config drops its label no longer keeps
  the label its earlier config had.
- **The canvas draws through the renderer's registry** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `frame()` draws every chart with `draw_registered` at the canvas display
  (`set_chart_display`, kept current by `resize`) instead of copying the
  series configs and building a list of draw inputs on every frame, and the
  wrapper no longer keeps its own GPU styles: edits check them with
  `preflight_chart_styles`, and the renderer builds them once per edit or
  resize. The JS API is unchanged.
- **`auto_fit_all` through the renderer** ([#160](https://github.com/shim9610/figgy/issues/160)).
  The wrapper calls the renderer's whole-chart fit instead of keeping its
  own series-extent cache, and no longer starts extent reductions when
  columns are uploaded or series change; a fit reduces what it has not seen
  and reuses the rest. The JS API is unchanged.
- **No panic paths** ([#152](https://github.com/shim9610/figgy/issues/152)).
  `hit_test` and `has_selection` throw a renderer error instead of
  aborting the module, `wait_submitted_work` rejects on a lost device, and
  an initialization progress error rejects the init promise. Document
  setters prepare the new state before they commit it, so a refused edit
  leaves the chart unchanged. Migration: callers that wrapped these in a
  `try` see the error there; others see the thrown error.
- **Shape annotations** ([#138](https://github.com/shim9610/figgy/issues/138)).
  `Rect`, `Ellipse` and `Polygon` items through the config JSON
  (SCHEMA.md); `annotations()` names them; `annotation_vertex(index, x, y)`
  answers a polygon's vertex handle (`"v3"`) and `drag_annotation` takes a
  vertex name beside the compass names, on `FiggyChart` and
  `FiggyBoundedChart` alike.
- **Parallel coordinates** ([#141](https://github.com/shim9610/figgy/issues/141)).
  `parallel` reads and writes in the config JSON and `"Parallel"` in
  `set_series` JSON (SCHEMA.md); the axis columns past the first two and
  the colour column are referenced columns; `auto_fit_all` fits each
  parallel axis from its columns' statistics; a picked row's
  `selection_details()` reports `values` (one per axis) and, mapped,
  `color_value`; each axis's tick labels hit and drag as
  `parallel_axis_labels_<i>`.
- **Sunbursts** ([#140](https://github.com/shim9610/figgy/issues/140)).
  A radial document's slices carry `children` (SCHEMA.md); `pick`, hover
  and `step_selection` answer the `{ group, index }` of a ring's node.
- **Colour images** ([#139](https://github.com/shim9610/figgy/issues/139)).
  `"RgbImage"` reads and writes in `set_series` JSON (SCHEMA.md); the
  three channel grids are referenced columns, and a picked cell's
  `selection_details()` reports `red`, `green` and `blue` beside the cell's
  position.
- **Hexagonal grids** ([#134](https://github.com/shim9610/figgy/issues/134)).
  `"grid_layout": "Hexagonal"` reads and writes in `set_series` JSON
  (SCHEMA.md); a picked cell's `selection_details()` reports the raised
  centre of an odd column.
- **Violins and rainclouds** ([#53](https://github.com/shim9610/figgy/issues/53)).
  A box plot series' `violin` and `points` read and write in `set_series`
  JSON (SCHEMA.md); a picked raw point's `selection_details()` reports its
  `index`, `position`, `value`, `direction` and category, and a violin
  without boxes reports its category.
- **3D surfaces** ([#49](https://github.com/shim9610/figgy/issues/49)).
  `set_config` / `get_config` read and write `"view_3d"` and `set_series`
  a heatmap's `"surface"` (SCHEMA.md, both omitted when absent).
  `zoom_view(factor)` zooms a 3D chart's camera, a data-area drag orbits
  it, `auto_fit_all` fits the z axis, and `selection_details()` adds a
  surface's `color_value` when it has a colour grid.
- **3D scatter and line series** ([#132](https://github.com/shim9610/figgy/issues/132)).
  `set_series` reads and writes a series' `"z_column"` (SCHEMA.md, omitted
  when absent) and `selection_details()` adds a point's `z` when its
  series has one. No new binding.
- **Ternary region fills** ([#49](https://github.com/shim9610/figgy/issues/49),
  [#55](https://github.com/shim9610/figgy/issues/55)). `set_series` reads and
  writes a ternary series' `"region"` (SCHEMA.md, omitted when absent); a
  fill picks as a point of its series. No new binding.
- **Ternary plots** ([#55](https://github.com/shim9610/figgy/issues/55)).
  `set_config` / `get_config` read and write `"ternary"` (omitted when
  absent), `set_series` the `"Ternary"` render type (SCHEMA.md);
  `selection_details()` adds a picked point's `c`, and `auto_fit_all` leaves
  a ternary chart's axes alone. No new binding.
- **Range bars** ([#54](https://github.com/shim9610/figgy/issues/54),
  [#52](https://github.com/shim9610/figgy/issues/52)). `RangeBars` series
  read and write in `set_series` JSON (SCHEMA.md); `selection_details()`
  reports a picked bar's `position`, `end`, `start` and, when present,
  `low`, `high`, `total` or `state` and `progress`, with its category on a
  categorical axis.
- **Streamlines** ([#51](https://github.com/shim9610/figgy/issues/51)).
  `set_series` / `get_series` read and write the `"Streamlines"` render
  type; `pick_point`, `step_selection`, the column references and
  `auto_fit_colorbar` (the colour column) cover it. No new binding;
  `SCHEMA.md` gives the form.

- **Offset spectra** ([#50](https://github.com/shim9610/figgy/issues/50)).
  `"line_offsets"` is read and written by `set_config` / `get_config` (on a
  subplot's `config` too) and omitted when absent; `pick_point`,
  `pick_data`, `selection_details` and `step_selection` report the
  unshifted values, `auto_fit_all` fits the members where they are drawn,
  and `hit_test` answers with the base layer's elements. No new binding;
  `SCHEMA.md` describes the key.

- **`autoLayout(configJson)`** ([#44](https://github.com/shim9610/figgy/issues/44)):
  a free function that returns the config JSON through `layout::auto_layout`,
  measured with the renderer's text measure for the config's draw style. It
  changes no chart; apply the result with `set_config`
  (`chart.set_config(autoLayout(chart.get_config()))`).
- **Adaptive log ticks.** `"log_ticks": "Adaptive"` is read and written on
  every axis of `set_config` / `get_config` and on `set_colorbar_axis`
  ([#42](https://github.com/shim9610/figgy/issues/42)); the default
  `"Decades"` is not written. No new binding; `SCHEMA.md` lists the values.
- **Math text.** `rich_text_from_tex(tex, font_size, font, r, g, b, a)`
  returns the JSON of `RichText::from_tex` for any rich text field of
  `set_config` or `set_series` ([#45](https://github.com/shim9610/figgy/issues/45)).
  `get_config` / `set_config` and `get_series` / `set_series` carry
  `math_node` and `math_parent`; a malformed tree fails at parse, before
  the chart changes. The bundled math face is part of the wasm binary, so
  no font needs to be registered for formulas.

- **Selection outlines.** `decoration_bounds(id)` returns `outline` (one
  polygon of `[x, y]` corners per drawn part, in physical canvas pixels)
  instead of `labels` for an element that outlines its parts
  ([#38](https://github.com/shim9610/figgy/issues/38)). Migration: read
  `outline` where `labels` was read.
- **Series extent key.** The series extent job identity had no vector
  `factor`, so a factor edit reused the previous extent; the per-field
  mutation test found it ([#46](https://github.com/shim9610/figgy/issues/46)).
- The wrapper's inline `mod web` is split into files under `src/web/`
  (moves only); no JavaScript export changes.
- **The bounded group, classified per chart**
  ([#40](https://github.com/shim9610/figgy/issues/40) item 13). Categorical
  bar, box plot and radar documents open in `FiggyChart`; `FiggyBoundedChart`
  is the radial chart's host. Migrations are under
  [Migrating from the bounded group](#unreleased-bounded-migration).
  - **Asynchronous radial picks.** `FiggyBoundedChart.pick`, `hover` and
    `hit_decoration` return Promises, because a radial chart picks its slices on
    the GPU ([#40](https://github.com/shim9610/figgy/issues/40) item 13b).
  - **Categorical documents in the Cartesian wrapper**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13d).
    `FiggyChart.set_categorical_document(json)` opens a `CategoricalChart`
    document as category bars (`Config::from_categorical`), registering its
    positions and value columns and fitting the value axis.
    `selection_details()` reports a selected bar's `category_id`,
    `category_label`, `value`, `direction` and `mode`, and `step_selection`
    walks the categories and the series of the bar group. `FiggyBoundedChart` no
    longer has the `"categorical"` family; creating one with it is an error that
    names the new method.
  - **Record and ragged registration**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13e).
    `FiggyChart.register_record(id, fields)` registers or replaces a box plot
    record from an object of `Float32Array`/`Float64Array` fields keyed by
    field name, and `FiggyChart.register_ragged(id, values, offsets)` a ragged
    column from two `Float64Array`s. Both are additive; nothing draws them
    yet (the box plot series of item 13f will).
  - **Box plot documents in the Cartesian wrapper**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13f).
    `FiggyChart.set_boxplot_document(json)` opens a `BoxPlotChart` document as
    box plot series (`Config::from_boxplot`), registering its positions column,
    one record per series and, for a series with outliers, its ragged outlier
    column from the borrowed summaries, and fitting the value axis (a tenth of
    the span as padding, nice ticks) unless the document fixes `value_range`.
    `selection_details()` reports a selected box's `category_id`,
    `category_label`, `index`, `direction` and its statistics read back from the
    record (`null` for a missing one), and `step_selection` walks the categories
    and the series of the box group (boxes, never outliers). `FiggyBoundedChart`
    no longer has the `"boxplot"` family; creating one with it is an error that
    names the new method.
  - **Radar documents in the Cartesian wrapper**
    ([#40](https://github.com/shim9610/figgy/issues/40) item 13g).
    `FiggyChart.set_radar_document(json)` opens a `RadarChart` document as radar
    series on a polar chart (`Config::from_radar`), registering its positions
    column and one value column per series from the borrowed values, and fitting
    the radius axis (from zero when no value is negative, nice ticks, the
    document's `ring_spacing` kept) unless the document fixes `value_range`.
    `selection_details()` adds a selected vertex's `category_id` and
    `category_label`, and `step_selection` steps the vertices of the series and
    across the radar series. `FiggyBoundedChart` no longer has the `"radar"`
    family; creating one with it is an error that names the new method.
  - **Value labels** ([#40](https://github.com/shim9610/figgy/issues/40) item
    13h). `pick_data` reports a label as `{ kind: "value_label", source_id,
    series_id, category_index, label_id, distance_px }`, and `pick_point` on
    a chart that draws value labels reports a label under the pointer before
    any point, as `{ source_id, series_id, point_index, distance_px,
    label_id }` with the category as `point_index`.
    `FiggyChart.press_value_label(label_id)` (and the element's
    `press_value_label`) selects the label and starts its drag; `on_move`
    writes the series' `labels.offsets` and `on_release` ends it.
    `step_selection` does not step a label; `selection_details()` reports its
    `category_id`, `category_label`, `text`, `offset` and `label_id`.
  - **The radial chart host** (13i). `FiggyBoundedChart` serves only the
    radial chart: its single-family document, renderer and frame enums and
    its unreachable host-clear path (a span is never drawn on a radial chart)
    are gone, and the Rust type is `FiggyRadialChart` in `src/radial.rs`,
    exported to JavaScript as `FiggyBoundedChart` with the same methods and
    the `"radial"` family argument. The `"categorical"`, `"boxplot"` and
    `"radar"` families stay errors naming the `FiggyChart` method for this
    release; an unknown family and a canvas below 160 × 160 name the radial
    chart in their errors. `FiggyChart.step_selection(dx, dy)` calls
    `Config::step_selection` (see [Model](#unreleased-model)); it steps as
    before, except that a column the wrapper has not registered counts as
    empty instead of trapping.

- **SVG and PDF export** ([#48](https://github.com/shim9610/figgy/issues/48)).
  `FiggyChart::export_svg(scale) -> String` and `export_pdf(scale) ->
  Uint8Array` export the document over the chart's clear colour, as
  `export_png`; `FiggyBoundedChart` (the radial chart) has the same two. The
  `<figgy-chart>` facade's `export_svg` / `export_pdf` hold the `"export"`
  operation gate and reject a chart shown from a stream before calling the
  kernel. The web demo (`index.html`) has SVG and PDF buttons beside the PNG
  one.

- **Broken axes** ([#200](https://github.com/shim9610/figgy/issues/200)).
  `set_config` takes an axis' `breaks` and `get_config` writes it back,
  leaving it out where absent; nothing else is new in the wrapper. A
  streamed chart with breaks is refused as the renderer refuses it.

- **Energy-level (Grotrian) diagrams** ([#203](https://github.com/shim9610/figgy/issues/203)).
  `set_config` and `set_series` take the new series kind, category rich
  labels and groups and the plot fill as JSON. `pick_point` answers an
  energy-level hit whole (its kind, row and, for a label, `part`); new
  `press_energy_label(series_id, part, index)` selects a label and starts
  its drag, which writes the row's `label_offset`. Group headers drag
  through the decoration routes like the other chrome.

- **Grotrian diagrams at atlas scale** ([#205](https://github.com/shim9610/figgy/issues/205)).
  `set_config` and `set_series` take the new keys (`category_window`,
  `breaks.sizing`, `bands`, `style.band`, a subplot's `marker`) as JSON. On
  the base layer every subplot's frame (`subplot_<i>:frame`) and its own
  decorations answer `hit_test` and `on_press` over the parent's: a press on
  a subplot's plot selects its frame, `on_move` drags the subplot by whole
  pixels, anywhere, and its handles resize it; the wrapper holds the display
  placement during the drag and `on_release()` releases it, so `on_release`
  now returns `Result` and throws on a renderer error. New
  `set_subplot_area(index, x, y, width, height)`, `subplot_area(index)`,
  `set_subplot_background(index, color)` and `insets()`, on `FiggyChart` and
  `<figgy-chart>`; `AxisPreset.Grotrian` is the new value `apply_axis_preset`
  takes. `pick_point` answers a band as its first level and a band
  label with `part: "band"`; `press_energy_label(series_id, "band", index)`
  drags it into the band's `label_offset`.

<a id="unreleased-bounded-migration"></a>

### Migrating from the bounded group ([#40](https://github.com/shim9610/figgy/issues/40) item 13)

The migrations of item 13 in one place; the model, renderer and browser
entries above describe the changes.

- **Readers.** `CategoricalChart`, `BoxPlotChart` and `RadarChart` are
  deprecated document readers for this release and are removed in the next
  milestone; code that names them compiles with a deprecation warning
  (`#[allow(deprecated)]` silences it while they last). Open a document with
  `Config::from_categorical`, `Config::from_boxplot` or `Config::from_radar`,
  which return the `Config`, its series and the `SeriesUpload`s that borrow
  the document's values. Upload them (the positions with
  `Renderer::add_column`; a bar group's value columns together with
  `add_columns`; each box plot record with `upsert_record` and the
  `BOX_PLOT_RECORD` schema and each outlier column with `upsert_ragged`;
  radar values with `add_column`), fit the value or radius axis with
  `Chart::auto_fit_layer_union` when the document fixes no range, draw with
  the Cartesian `Renderer` and pick with `pick_chart_data` (a bar or a box
  answers `PickedData::HistogramBin` with its category row, a radar vertex
  `PickedData::Point`, a value label `PickedData::ValueLabel`). In the
  browser, `FiggyChart.set_categorical_document`, `set_boxplot_document` and
  `set_radar_document` do all of it.
- **Removed renderers, types and functions.** `CategoricalRenderer`,
  `CategoricalFrame` (`bar_rect`, `hit_test`, `decorations`, …),
  `CategoricalError`, `BoxPlotRenderer`, `BoxPlotFrame` (`hit_test`,
  `part_rect`, `box_rect`, `plot_rect`, `decorations`, …), `BoxPlotPick`,
  `BoxPlotPart`, `BoxPlotError`, `RadarRenderer`, `RadarFrame` (`hit_test`,
  `part_rect`, `vertex_position`, `plot_rect`, `plot_circle`,
  `decorations`, …), `RadarPick`, `RadarPart`, `RadarError`, their shaders
  and layout modules and the bounded chrome only they used;
  `decoration::DecorationMap`, `BoundedAnnotationFrame::hit_test_active_layer`,
  `FrameBackground`, `RadialRenderer::set_background` and `background`,
  `RadialFrame::background`, `BoundedAnnotationFrame::draw_underlay` and
  `draws_underlay`, `RadialFrame::hit_test` and `hit_test_at_rest`;
  `CategoricalChart::segments`, `CategoryBarSegment`, the three documents'
  `label_text`, `CategoricalChart::bar_override`, their `reorder_categories`,
  `set_active_layer` and `normalize_title`, `RadarChart::is_emphasized` and
  `resolved_style`, and `DecorationElement.followers`. The
  `categorical_editor`, `boxplot_editor` and `radar_editor` examples are
  removed; `bounded_titles` is `radial_titles`.
- **The radial chart.**
  - Hit-test decorations with `frame.decoration_at(&chart, p)` instead of
    `frame.decorations().hit_test(p)`, name a hit with
    `frame.decoration_id(&chart, hit)` instead of `map.element(hit).id`, and
    drag with `frame.drag_decoration(&mut chart, hit, dx, dy)` instead of
    `map.drag_by(&mut chart.decorations, hit, dx, dy)`: `decorations()` is
    a `HitMap<RadialChart>`, queried with the chart the frame was prepared
    from.
  - Pick slices with `frame.pick(p)?.resolve().await?` (natively
    `resolve_blocking()`) instead of `frame.hit_test(p)`; `pick` takes
    `&Arc<RadialFrame>`.
  - Call `format_labels()` after building or editing slices, values, names
    or label formats, and `normalize_labels()` on a loaded document: the
    renderer draws `RadialSlice::text` and refuses a shown label without
    it. An exhaustive `RadialSlice` literal needs `text: None` (or use
    `RadialSlice::new`).
  - Build a radial `chart_title` as a `ChartTitleOptions` (zero offsets and
    band) where a `BoundedTitleOptions` was built.
  - Draw the annotation layer after the frame with `draw_overlay` alone;
    drop `set_background(FrameBackground::HostClear)` and `draw_underlay`.
    The radial chart has no value axes, so a span was never drawn on it.
- **Rust literals and matches.** `Config { .. }` adds `bar_groups: None`,
  `line_offsets: None` and `ternary: None` (a subplot's `config` likewise);
  a `data_render::ScatterTransform` literal adds `ternary_lt` and
  `ternary_r` (zeros on a rectangular chart);
  `AxisOptions { .. }` adds `categories: None` and `cross_at: None` (an
  exhaustive destructuring names both); `PolarOptions { .. }` adds
  `grid_shape: None`; a `RadarSeriesStyle` literal that names every field
  adds `fill_opacity: None`; the three series label types add
  `offsets: Vec::new()`; `data_render::SeriesLayers` adds
  `value_labels: None`; a `DecorationElement` literal drops `followers`
  (`..Default::default()` covers the additions). Exhaustive matches add
  `DataRenderType::{CategoryBars, BoxPlot, Radar, RangeBars, Ternary}`,
  `UploadValues::{BoxRecord, BoxOutliers, Dense}`,
  `PickedDataRef::ValueLabel` and `PickedData::ValueLabel`.
  `layout::axis_anchor(side, plot, cross)` takes the plot as
  `&RectF::from_rect(&data_area)` and a crossing value (`None` for the
  former anchor).
- **Web API.** `FiggyBoundedChart` keeps its JavaScript name, its methods
  and the `"radial"` family. `pick`, `hover` and `hit_decoration` return
  Promises: `await` them. A parsed radial document's slices without `text`
  get the text their label format writes, so remove or rewrite a set `text`
  after editing a value or a format. Replace
  `FiggyBoundedChart.create(canvas, "categorical" | "boxplot" | "radar",
  json, w, h)` with a `FiggyChart` and `set_categorical_document(json)`,
  `set_boxplot_document(json)` or `set_radar_document(json)`; the marks are
  picked with `pick_data` and selected through `picked_data`. The three
  family names are errors naming those methods for this release.
- **Look and behaviour changes.** A legacy document opened through a reader
  draws with the Cartesian or polar chrome instead of the former bounded
  layout: Cartesian margins (an explicit `decorations.data_area` is not
  carried), the Cartesian legend box, the export clear colour instead of
  `background`, the Cartesian selection outline or the polar picked ring
  instead of a brightened series, and value labels through the contour
  label module; the marks themselves keep their places and shading (the
  per-case pixel counts are under [Renderer](#unreleased-renderer)). A legacy
  radar document's `style.emphasis_brightness` is kept in the document but
  not drawn. The radial title moves about 2 px down into its band and is
  clipped instead of refused when wider than the canvas, and a selected
  outside radial label outlines its text lines and its leader line.
- **Box plot statistics (D6).** The renderer cannot read a record on the
  CPU, so it clamps unordered box statistics instead of rejecting them: the
  body spans `min(q1, q3)..max(q1, q3)`, the median is clamped into it, the
  stems reach the body, a notch needs both CI endpoints and a row with a
  missing required field draws nothing. `BoxPlotChart::validate` still
  rejects an unordered document, so a document drawn through the reader
  draws as before; a host that uploads its own records checks the order
  itself.
- **Stacked bar fit.** A `Stacked` bar group's value-axis fit comes from
  the per-column statistics, so its columns may be uploaded in any batches.
- **Decoration oracle exclusions.** Item 13 adds three rows to the decoration
  oracle's exclusions: I5 of the box plot scene's `axis_title_left` and I5
  of the radar scene's `tick_labels_left`, both sub-pixel glyph snapping
  between 1x and 2x (box-filtered shares of 63 % and 52 % against a 50 %
  limit), and I7 of the radar scene's `axis_bottom`, the polar outer ring's
  orphan ink the polar scene is excluded for. The radial leader-line row is
  gone (13b), and the radial scene has no exclusion.

### Tests and guards

- **The field contour and violin picks compile in a browser's software
  GPU.** Creating the `field contour data pick` and `violin data pick`
  pipelines grew Chromium's SwiftShader GPU process past 12 GB, so a
  browser without a GPU lost the device at its first pick (the field
  pipeline also overflowed Windows WARP's stack, #226). The polar
  foot-point iteration (#208) and the binary searches inside it were
  nested in the contour pick's loop over the levels; the pick now runs
  each level's steps in one flat loop, through
  `contour_foot_begin` and the foot-point step the paint and the
  highlight share (now the walk of the next entry). `locate` keeps the bracket's boundaries as it bisects
  and returns them (`CellHit.lo_t`, `hi_t`), so `contour_sample` no longer
  transforms four boundaries again for the cell's span. Distances and
  picks are unchanged. The violin pick's segment search keeps the two
  points it brackets instead of reading them again, and reads both
  neighbouring points through one call. In SwiftShader the contour pick
  now builds in about 60 s at 850 MB and the violin pick in about 90 s at
  530 MB; the cell pick goes from 17 s to 9 s and the contour paint from
  8 s to 4 s.

- **Contours draw in a browser's software GPU.** SwiftShader compiles a
  fragment entry when it first draws, not when its pipeline is created, so
  measuring pipeline creation missed the contour paint: the first draw of
  a contour (`fs_contour`, `fs_contour_labelled`, the 3D floor contour and
  its ID pick) grew Chromium's GPU process past 12 GB and lost the device.
  This is what a page creating several contour charts, and an export after
  a full prewarm, ran into. The paint nested the polar foot-point loop in
  its loops over the level blocks and candidates, and every foot-point
  step held two binary cell searches. The paint now takes the blocks, the
  candidates and the foot-point steps in one flat loop, like the pick. The
  steps are a walk (`contour_foot_walk`, `contour_foot_advance`) that the
  paint, the pick and the selected-level highlight advance one iteration
  of their loop at a time. `locate` is split into `locate_begin`,
  `locate_step` and `locate_end`, and `contour_sample` into the search and
  `contour_sample_cells`, so the walk halves one cell search per
  iteration, both axes through one call site. Pixels and picks are
  unchanged: on Apple M4 (Metal) the exported images of polar and
  rectangular contours at 8 and 64 levels, 1x and 2x, are byte-identical
  before and after. A polar contour export takes 1 to 5 ms (3 to 8%)
  longer there; a rectangular one is unchanged. In headless Chromium 151
  (SwiftShader, Apple M4) the first use of each contour entry now takes 4
  to 5.5 s at under 210 MB, the selected-level highlight 9 s instead of
  30 s, and creating the contour pick 6.5 s at 200 MB instead of 27 s at
  710 MB. Draws after the first take 2 to 3 ms either way.

- **Shaders a browser refuses.** Chrome's WGSL compiler refused eight
  modules that naga accepts (the line, bar and surface draw modules and
  the five GPU pick modules), so their pipelines failed in the browser:
  a constant that evaluates to NaN (`bitcast<f32>(0x7fc00000u)`, the
  parallel-coordinate ends, the box-plot value label's missing top and a
  ternary pick past its third column), a literal past `f32::MAX`
  (`3.4028235e38`, the raincloud member check), and a `fwidth` after a
  return that depends on the fragment (the surface's picked patch). Each
  now carries validity by an explicit flag or a bit test, and the patch
  colour is computed before the choice. `shader_assembly`'s
  `no_assembled_module_holds_a_constant_a_browser_refuses` scans every
  assembled module for the first two forms. None of these shaders was in
  a release.

- **Older documents are checked by what they draw** ([#160](https://github.com/shim9610/figgy/issues/160)).
  `tests/released_documents.rs` no longer requires the released bytes back:
  each released document must parse, read back as the same value after a
  save, and re-serialize unchanged from the save on.
  `tests/released_documents_render.rs` exports every released chart
  document (configs, radial charts, and the categorical, box plot and radar
  documents through their readers) at 1x and 2x before and after the save
  and requires identical pixels; an empty default document must be refused
  for the same reason on both sides, and an edited one must draw. The edited
  radial documents' explicit plot area starts above the 70 px title band
  every release has used, so no release draws them as written: they must
  be refused alike, and their pixels are compared with the automatic plot
  area. CLAUDE.md states the rule.

- **Streamlines** ([#51](https://github.com/shim9610/figgy/issues/51)).
  `vector_render` places the heads at fixed arc spacing along the tangent,
  skips NaN gaps, keeps their pixel spacing at 1x and 2x and under pans,
  colours line and heads through the colourbar, holds the slot draw to
  `ARROW_VERTICES`, draws the picked ring, and scales streamlines by an
  overlay axis and inside a subplot; `pick_draw_agreement` holds stroke and
  heads to their paint (solid and dashed, plain and colour-mapped, linear
  and logarithmic y); the model, the registration and SSOT tests cover the
  rules, the round trip, `step_selection`, the polar and streaming
  rejections and edits of `arrows` and `color_map`. The playground catalogue
  gains a `Streamlines` scene (a host-integrated dipole with a logarithmic
  speed colourbar), which the decoration oracle sweeps in both passes with
  no exclusion.

- **Offset spectra** ([#50](https://github.com/shim9610/figgy/issues/50)).
  The playground catalogue gains an `OffsetSpectra` scene (twelve stacked
  Gaussian spectra, the first filled to its baseline, a skew along x and a
  label per trace as an annotation), which the decoration oracle checks on
  both sweeps with no exclusion row and the playground selftest exports
  and edits. `pick_draw_agreement::offset_lines_pick_where_their_shifted_parts_paint`
  holds every member's painted pixels to its picks (a filled line, a dashed
  line with markers and error bars, a log axis stacked by decades, 1x and
  2x); `line_area_step::offset_lines_report_unshifted_values`, the renderer's
  `offset_lines_export_at_1x_and_2x`, `offset_line_ring_sits_on_the_shifted_point`,
  `offset_picks_take_the_painted_member_first`, `pan_keeps_offset_picks_on_the_drawn_trace`,
  `offset_members_shift_on_their_overlay_axis`, `auto_fit_includes_member_offsets`,
  `streamed_series_reject_line_offsets`, `polar_rejects_line_offsets` and
  `subplot_members_draw_shifted_in_their_own_area` compare each with a host
  that shifted the data itself; the model's `step_selection_ignores_line_offsets`,
  `line_offsets_round_trip_and_old_documents_unchanged`,
  `line_offset_members_expand_into_shifted_panels` and
  `subplot_line_offsets_are_its_own`; the SSOT case
  `"line_offsets (None -> Some)"`, an `offsets` suite of the resident display
  key mutations, and a browser test of the round trip, hit test, pick and
  selection details.

- **`auto_layout`** ([#44](https://github.com/shim9610/figgy/issues/44)). The
  playground catalogue gains a `LongLabels` scene (`0.00300` labels under a
  math Y title, negative long decimals, a logarithmic top axis with adaptive
  labels, timestamps and a colourbar with long labels), which the decoration
  oracle checks as laid out with its overlap excluded for #44. A second
  oracle sweep runs every Cartesian scene through `auto_layout` and holds it
  to I1 to I7 (`auto_layout_i1` … `auto_layout_i7`). `tests/auto_layout.rs`
  checks purity and idempotence on every scene, untouched data fields, the
  bands of the default config and the released config documents (every
  change listed), a serde round trip and
  `set_chart_config`; the model unit tests pin the band rule, offsets,
  formats, the colourbar, the legend, overlays and subplots with a stub
  measure, and the web crate `autoLayout`. With #270 they also cover a band
  shrinking to its labels, the chart title's padding and a title-less chart,
  the perpendicular labels' overhang, category group headers, a hidden frame
  and a ternary chart.
- **Adaptive log ticks** ([#42](https://github.com/shim9610/figgy/issues/42)).
  The model pins the plan's golden ladders, majors and minors (with guarded
  and extreme ranges), a sweep where every `Decades(n)` ladder draws the
  default policy's majors and minors at `major_spacing = n` and every
  promoted 2·10^k or 5·10^k major sits on the decade ladder's minor,
  `log_multiple` over decades -307 to 307, `LogMinors` against the former
  walk, the wire format and the link and overlay copies. The renderer pins
  the labelled texts of nine ranges × {120, 240, 480, 960} px × side × four
  formats (the same inverted, and at 2x except three named cells whose
  plain digit boxes sit on the half-em threshold), the ladder-or-stride shape with
  half-em gaps, recovery on resize, zoom and font changes, the warning
  skip, a colourbar, polar rings and ray, a 2e4 label's bounds, hit and
  drag on every side and on overlay and subplot axes, the zoom
  3e3..8e5 → 1e4..8e4 → 1e4..1e8 through `set_y_range` and
  `set_chart_view_state` with PNG export at 1x and 2x, scale consistency,
  the SSOT edits, and a browser round trip with a hit and drag.
- **Math text** ([#45](https://github.com/shim9610/figgy/issues/45)).
  `tests/math_text.rs` checks a formula as chart title, axis title (rotated
  too), legend label and colourbar title at 1x, 2x and 3x: ink inside the
  bounds, the hit test, the selection box and a drag. The catalogue scenes
  carry math titles, an overlay title, a subplot title, an annotation and a
  radar title, so the decoration oracle checks math on every invariant.
  `decoration_sinks_guard` lists `text_render/math_layout.rs`;
  `ssot_invalidation_tests` covers a node's kind, a child's slot and text
  and a valid id renumbering; the model tests cover every error kind, the
  pinned wire form of every kind and the TeX subset.

- **Flag blend lint.** `shader_consistency` rejects a choice by a 0/1 flag
  that drivers may evaluate inexactly: an unguarded `mix` by a flag, a flag
  as a factor of a hand-written blend (`x + f * (y - x)`,
  `(1.0 - f) * x + f * y`) and a `select` by a log flag. The flags it checks
  (`scale_log`, `polar_angle.x`, `is_log` and `select(0.0, 1.0, …)` locals)
  are listed in SHADER_COMMON §3.1
  ([#47](https://github.com/shim9610/figgy/issues/47)).
- **Duplicate guards** ([#46](https://github.com/shim9610/figgy/issues/46)).
  `every_wgsl_function_is_defined_once_outside_the_allowlist` fails on a
  WGSL function defined in more than one product shader, except the
  per-variant bodies the assembler chooses between (`ASSEMBLED_ALTERNATIVES`);
  `model_struct_hand_offs_name_every_field_outside_the_allowlist`
  fails on a `..` rest pattern on a model type in product code; per-field
  mutation tests walk every serialized leaf of a document and fail when a
  cached output changes while its key does not (they found the cache-key
  gaps above). `every_assembled_module_parses_and_validates`,
  `wgsl_constants_equal_their_rust_constants` (the shared `u32` and `f32`
  constants) and `mirrored_struct_layouts_match_their_rust_twins` replace
  the copy-sync tests.
- **Test suite** ([#47](https://github.com/shim9610/figgy/issues/47)). 20
  duplicate tests are removed and 7 pairs merged, each keeping every check;
  the draw-and-pick agreement tests probe the pixels of an edge band around
  each primitive. Section 3: the five P-00 streaming feasibility probes
  (`stream_*_replay`, `stream_field_axis_probe`) are removed with the frozen
  streaming paths they probed; the model crate's serde checks run as one
  test per file (`config.rs`, `data_config.rs`), each former test a step
  that keeps every assertion; the visual probes are gone and the images
  `ci/verify.py` collects are written by the pixel tests that draw the same
  scenes (histogram, heatmap, contour), while the colourbar, layout
  geometry and demo field panel probes became pixel tests: the ramp is
  painted beside the data area on its side and runs from the axis minimum,
  the selection highlight changes pixels around the bar only, the axes
  frame the data area and the legend's ink lies in its corner's quadrant,
  and the demo panel's fill, level rings and labels are checked where the
  Gaussian puts them.
- **Dev-dependencies.** `figgy-renderer` and `figgy` gain a `serde`
  dev-dependency for the mutation walker. `figgy-renderer` gains the
  `cpu-twin` feature (`renderer::pick::cpu_twin`, test support, hidden from
  the docs), which its own tests enable through a dev-dependency on the
  crate itself; a packaged crate builds its tests without it. No product
  dependency changes.
- **The bounded group, classified per chart**
  ([#40](https://github.com/shim9610/figgy/issues/40) item 13).
  - **Released fixtures** (13a). Released-document fixtures written by
    figgy-model 0.12.1 cover the radial, categorical, box plot and radar
    charts at their default and edited (`released_documents`).
  - **Radial pick and decorations** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13b). `radial_render` compares the GPU pick with the CPU twin over
    tilt, depth, gaps, corners, explode, the hole, the detail circle and the
    lift, lifted and at rest, and checks the at-rest pick; the shader mirrors
    hold both radial modules' `Sector` and the pick query and result to their
    Rust twins. The oracle's radial branch and the radial interaction tests
    go through the radial frame's `HitMap<RadialChart>` (a test-support
    adapter covers both hit-map APIs), and the selection-only check accepts a
    selection that outlines its parts.
  - **Category bars** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13d). `category_bars_render` checks every mode in both directions at
    1x and 2x against the bars' expected extents, signed and missing stacks,
    overrides and materials, the selection outline of an overridden bar, the
    value-axis fit from the per-column statistics (the sum rule, its bound
    when the series peak in different categories, one batch or several),
    the axis crossing, an overlay's and a subplot's own bar groups
    drawn and picked on their layers, and a released document drawn through
    the reader; `pick_draw_agreement` holds the bar pick to the painted bar;
    `released_documents` converts every released categorical document and
    round-trips the result; the SSOT matrix classifies `bar_groups` and
    `cross_at`, and the resident-body mutation walk covers a stacked category
    bar panel (385 config and 82 series leaves); the decoration oracle and
    the playground sweep the categorical scene as a Cartesian scene; a
    browser test opens a document in `FiggyChart`. The legacy renderer's
    `categorical_render` and `categorical_browser` are removed with it; the
    bounded annotation tests that drew on bars run on box plots.
  - **Box plots** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13f). `box_plot_render` checks the boxes' statistics in both
    directions at 1x and 2x against their expected pixel extents, notches,
    missing boxes and clamped unordered statistics, overrides and materials
    restyling only their box, the selection outline of an overridden box, the
    value-axis fit over the record and outlier statistics, a replaced record
    and mismatched sources, an overlay's and a subplot's own box groups, the
    annotation layer's spans and reference lines over the categories and
    values, a 2x export that is the 1x layout doubled, and a released
    document drawn through the reader;
    `pick_draw_agreement::box_plots_pick_the_box_they_paint` holds the pick of
    every part to the painted box; `released_documents` converts every
    released box plot document and round-trips the result; the resident-body
    mutation walk covers a box plot panel (384 config and 166 series leaves,
    records and a ragged column included); the decoration oracle and the
    playground sweep the box plot scene as a Cartesian scene (the oracle's
    I5 rotated-title 1x/2x check excludes the box plot's left axis title,
    whose sub-pixel centroid offset the categorical scene shares, #40); a
    browser test opens a document in `FiggyChart`, selects, steps and exports
    it; `streamed_series_reject_box_plots` checks the streaming refusal. The
    legacy renderer's `boxplot_render` and `boxplot_browser` are removed with
    it, and so are the box plot cases of the bounded interaction and label
    layout tests and the bounded span underlay test (radial and radar charts
    place no spans); the annotation hit-region test that drew on boxes runs
    on a radar chart.
  - **Radar series** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13g). `radar_series_render` checks the polygon, its closing segment
    and fill, the zero angle and direction, a NaN vertex, a sector, several
    series, circular and polygon grids, the category spoke labels, the picked
    ring, the radius fit, the validation errors, the annotation layer, a
    polar subplot, SSOT edits against a fresh renderer and a released
    document drawn through the reader;
    `pick_draw_agreement::radar_polygons_pick_their_outline_closing_segment_markers_and_fill`
    holds the pick of every part to the painted polygon; `released_documents`
    converts every released radar document and round-trips the result; the
    resident-body mutation walk covers a radar panel; the decoration oracle
    and the playground sweep the radar scene as a polar scene (the oracle
    excludes the outer ring's I7 orphan ink, as on the polar scene, and the
    ring labels' I5 2x check, whose ink centroids sit up to 0.64 px apart at
    1x and 2x, the glyph snapping the box plot title exclusion describes;
    the reader hides the ray line, so the polar ray's I6 row does not
    arise); a browser test opens a document in `FiggyChart`,
    selects, steps and exports it; `streamed_series_reject_radar` checks the
    streaming refusal. The legacy renderer's `radar_render` and
    `radar_browser` are removed with it, and so are the radar cases of the
    bounded interaction, annotation and decoration tests; the annotation
    hit-region test that drew on a radar chart runs on a radial chart.
  - **Value labels** ([#40](https://github.com/shim9610/figgy/issues/40) item
    13h). `value_labels_render` checks, at 1x and 2x, that bar, box and radar
    labels sit where the former charts placed them (within `2 * scale + 1`
    px), that the drawn label is the SSOT string's own raster texel for
    texel, that an empty string draws nothing, that a drag moves the label
    by its offset alone, that the selected label's box is its frame, that
    overlay and subplot series draw their labels in their panels, that a
    label at the data area's edge flips to the other side of its anchor for
    every kind, drawn whole and picked there, and the typed pick of a label.
    `pick_draw_agreement` gains
    `value_labels_pick_where_they_paint`; the atlas key is mutation tested
    (`the_value_label_atlas_key_names_every_bake_input`); the model tests
    cover the offsets' validation, drag and round trip and the label id; the
    browser tests cover the label hit's JSON and a label's pick, selection
    and drag (`a_value_label_picks_selects_and_drags`). The playground scenes
    of the box plot and radar charts show their labels.
  - **The radial tests** (13i). The tests and test support that only test the
    radial chart are named for it: `bounded_interaction`,
    `bounded_label_layout`, `bounded_interaction_browser` and
    `bounded_area_browser` are `radial_interaction`, `radial_label_layout`,
    `radial_interaction_browser` and `radial_area_browser`, and
    `support/bounded_decorations.rs`, `bounded_selection.rs` and
    `bounded_area.rs` are `radial_decorations.rs`, `selection_only.rs` and
    `radial_area.rs` (the area check without its one-family macro, the
    decoration adapter without the always-empty followers). The decoration
    oracle's bounded branch is a radial sweep without the per-family trait,
    its rows unchanged: the radial scene has no exclusion. The model tests the
    moved `Config::step_selection` (points across series, a point vector,
    histograms by orientation, category bars and boxes by layer, matrix and
    grid vector cells, no selection, labels); the tests of the removed
    helpers and underlay go with them.

- **SVG and PDF export** ([#48](https://github.com/shim9610/figgy/issues/48)).
  `tests/vector_export.rs` with `support/svg_parts.rs` and
  `support/pdf_structure.rs`: every catalogue scene at 1x and 2x in both
  formats reads back, element by element and in order, as the parts the
  export was handed (a `Parts` recording of the same passes, within
  0.01 px, text by its block origin and rotation and each glyph pen); the
  PDF's header, cross-reference offsets, trailer, object graph,
  ExtGStates, soft masks and inflated content are checked, and its images
  are the SVG's pixels. With every decoration hidden each data image is the
  PNG export's crop byte for byte; with decorations shown the grid raster,
  data image and decoration raster composite to the PNG export within 1/255
  but for at most 0.05 % of the channels (8-bit blend rounding, under
  3/255). Size budgets (256 KiB SVG and 128 KiB PDF outside the images,
  images within 1.05 times the PNG), determinism, the clear colour, a
  streamed source, the sketch and Milkyway styles and the radial chart are
  covered. Node tests cover the facade's two methods and their rejection
  during stream replay; the playground has SVG and PDF export buttons and
  its selftest exports every scene in the three formats.

- **Broken axes** ([#200](https://github.com/shim9610/figgy/issues/200)).
  `tests/axis_break_render.rs` checks, at 1x and 2x and in both styles
  where they differ: markers on every piece where the chrome places them
  (both axes broken, either inverted, a log axis cut in decades); nothing
  in a gap from a line, a fill, a highlight or an annotation; a `Panels`
  line keeps its slope per piece; a cut outside the range changes no
  pixel; picks per piece and none in a gap; the screen path draws what
  the export does; overlays, subplots and line offset members on broken
  axes, their gaps on their owner's; the picked highlight on its piece;
  spans and reference lines per piece; and the streamed refusal. The
  model tests cover the break map, validation, serde, the chrome, label
  thinning, `mirrored_breaks`, member cuts and annotation clipping; the
  SSOT suite every break edit (ranges, style, gap, mark, frames, an
  overlay's breaks). The playground gains a **Broken axes** scene (both
  styles, a log Y cut), which the decoration oracle sweeps, and the
  browser tests a broken axis' JSON round trip and its upper labels'
  hit and drag. Each new check was mutation tested.

- **Energy-level (Grotrian) diagrams** ([#203](https://github.com/shim9610/figgy/issues/203)).
  `tests/energy_levels_render.rs` checks at 1x and 2x: placement of
  levels, limits and arrows; the picked level drawn wider; a Continuous
  break; a components view drawing its source's records; 5,004 levels and
  1,500 transitions drawn with no count limit (with their export time);
  picks of levels, arrows and labels and none in a gap; a series edit
  redrawing a registered chart; a document round trip drawing the same
  pixels; SVG and PDF exports; and an inset covering its parent's data
  and annotations and taking the pointer (mutation tested by drawing it in
  the old order). The model tests cover validation, serde, stepping,
  label drags, the continuum, category groups (validation, hit, drag,
  serde) and rich labels; the SSOT suite classifies the new fields. The
  playground gains an **Energy levels** scene, swept by the decoration
  oracle, the auto layout and vector export tests.

### Documentation

- **Streamlines** ([#51](https://github.com/shim9610/figgy/issues/51)).
  `CONFIG.md` (in Korean) gains the streamline section next to the vector
  field one, `SCHEMA.md` (in Korean) the render type's form and its serde
  rules, and `SHADER_COMMON.md` the arc slots, the mapped stroke, the path
  form and the arrowhead pick.

- **Offset spectra** ([#50](https://github.com/shim9610/figgy/issues/50)).
  `CONFIG.md` describes `LineOffsets` (the Korean section in Korean),
  `INTERACTION.md` the panels, the pick order, selection, fit and export, and
  `SCHEMA.md` (in Korean) the `line_offsets` key.

- **`auto_layout`** ([#44](https://github.com/shim9610/figgy/issues/44)).
  `API.md` lists it among the convenience helpers, `CONFIG.md` points the
  label section to it, `README.md` lists it under embedding, and the
  playground README describes the scene and the "Auto layout" button. The
  Korean documents carry the same sentences in Korean: the Korean section of
  `CONFIG.md`, `WASM.md`, `SCHEMA.md` and `README.ko.md`.
- **Adaptive log ticks** ([#42](https://github.com/shim9610/figgy/issues/42)).
  `CONFIG.md` describes `AxisOptions::log_ticks` and the `major_spacing`,
  `minor_count` and `link_x` rows that it changes (the Korean section in
  Korean), `API.md` the range helpers and label density, `INTERACTION.md`
  the adaptive labels as their side's label group, and `SCHEMA.md` (in
  Korean) the JSON values and the editing rule.
- **Browser product status**
  ([#57](https://github.com/shim9610/figgy/issues/57)). The documentation describes
  `crates/web` (`figgy`: `FiggyChart`, `FiggyBoundedChart` and `<figgy-chart>`)
  as figgy's browser product, maintained with the renderer, instead of an
  integration example. The package keeps its own version and `publish = false`;
  its publication target is still open. No API, serialized field or build
  change.
- `SHADER_COMMON.md` is the contract of the shared fragments (layouts,
  bindings, invariants, the flag table of §3.1) and names the file that
  holds each block; it is no longer parsed by a test. The contributing
  guide, the CI guide and the guides that named a split file point at the
  file the item lives in now.
- **The bounded group, classified per chart**
  ([#40](https://github.com/shim9610/figgy/issues/40) item 13).
  - **Categorical bars** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13d). `CATEGORICAL.md` (Korean) says that the categorical chart is a
    `CategoryBars` series on a `Config` with a categorical axis, read from
    legacy documents by `Config::from_categorical`, and drops the sections of
    the removed bounded renderer; `SHADER_COMMON.md`
    §2.4 and §5.13 hold the category bar contract; `CONFIG.md`, `API.md`,
    `INTERACTION.md`, `WASM.md`, the browser `README.md` and `SCHEMA.md` and
    the model README describe the series kind, `bar_groups`, `cross_at` and
    `set_categorical_document`, the Korean sections in Korean. The model README's DataLess class is the
    radial chart alone.
  - **Box plots** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13f). `BOXPLOT.md` describes the `BoxPlot` series, its record and
    ragged sources, the box group, the clamping, styles and overrides, value
    labels, the fit, picking and the reader, and drops the removed renderer;
    `SHADER_COMMON.md` §2.5 and §5.15 hold the box plot contract; `CONFIG.md`,
    `API.md`, `INTERACTION.md`, `WASM.md`, `USAGE.md`, `ARCHITECTURE.md`,
    `GALLERY.md`, the browser `README.md` and `SCHEMA.md`, the model README,
    the example and playground READMEs and the top-level READMEs describe the
    series kind and `set_boxplot_document`. The Korean text (the Korean
    sections of `BOXPLOT.md`, `CONFIG.md`, `INTERACTION.md`, `USAGE.md`,
    `ARCHITECTURE.md` and `WASM.md`, `SCHEMA.md`, `README.ko.md` and
    `examples/README.md`) is updated in Korean.
  - **Radar charts** ([#40](https://github.com/shim9610/figgy/issues/40)
    item 13g). `RADAR.md` describes the `Radar` series on a categorical
    angle axis, its style, labels, picking, the fit and the reader, and drops
    the removed renderer; `POLAR.md` and `CONFIG.md` describe the categorical
    angle axis and `grid_shape`; `API.md`, `INTERACTION.md`, `WASM.md`,
    `USAGE.md`, `ARCHITECTURE.md`, `GALLERY.md`, the browser `README.md` and
    `SCHEMA.md`, the model and renderer READMEs, the example, playground and
    CI READMEs, the contributing guide and the top-level READMEs describe the
    series kind and `set_radar_document`. The Korean text (the Korean
    sections of `RADAR.md`, `POLAR.md`, `CONFIG.md`, `INTERACTION.md`,
    `USAGE.md`, `ARCHITECTURE.md` and `WASM.md`, `SCHEMA.md`, `README.ko.md`
    and `examples/README.md`) is updated in Korean. `gallery-radar.png` and
    `gallery-radar-circle.png` keep the former renderer's images, with value
    labels, until 13h draws them.
  - **Value labels** ([#40](https://github.com/shim9610/figgy/issues/40) item
    13h). `INTERACTION.md` (Value labels), `API.md`, `WASM.md`, `CONFIG.md`,
    `CATEGORICAL.md`, `BOXPLOT.md`, `RADAR.md`, `GALLERY.md`,
    `examples/README.md`, `crates/web/README.md`, `crates/web/SCHEMA.md` and
    `SHADER_COMMON.md` (§5.16 Label quad, §5.17 Value labels) describe the
    drawn labels, their pick, selection and drag. The categorical, box plot
    and radar gallery images are regenerated with their labels.
  - **The end state** (13i). The guides describe the radial chart as the
    DataLess chart, `FiggyBoundedChart` as its host and the three legacy
    documents as deprecated readers, and drop the deleted underlay path,
    `DecorationMap` and the removed helpers: `ARCHITECTURE.md` (both
    languages), `INTERACTION.md`, `API.md`, `WASM.md` (section 8 is the radial
    chart host; its anchor is unchanged), `CONFIG.md`, `RADIAL.md`,
    `RADAR.md`, `BOXPLOT.md`, `CATEGORICAL.md`, the renderer, model, browser,
    example, playground, fixture and CI READMEs, `SCHEMA.md`,
    `SHADER_COMMON.md`, the contributing guide and the top-level README. The
    Korean text (the Korean sections of `ARCHITECTURE.md`, `INTERACTION.md`,
    `CONFIG.md`, `RADIAL.md`, `RADAR.md`, `BOXPLOT.md` and `WASM.md`,
    `CATEGORICAL.md`, `SCHEMA.md` and `examples/README.md`) is edited in
    Korean.

- **SVG and PDF export** ([#48](https://github.com/shim9610/figgy/issues/48)).
  `USAGE.md` (both languages) describes the API, the page size and the
  omissions, `ARCHITECTURE.md` (both languages) the export pipeline's two
  halves and the vector passes, `WASM.md` the browser methods and their
  streaming rejection, `INTERACTION.md` what the files hold of each
  interaction, `RADIAL.md` the radial export, `API.md` the return types. The
  Korean text (the Korean sections of `USAGE.md` and `ARCHITECTURE.md`, and
  `WASM.md` and `RADIAL.md`) is written in Korean.

- **Broken axes** ([#200](https://github.com/shim9610/figgy/issues/200)).
  `CONFIG.md` (both languages) gains the `breaks` row and the
  `AxisBreaks` section, `SCHEMA.md` (in Korean) the JSON form and its
  rules, and `SHADER_COMMON.md` the `Transform` break table and the cut
  remap.

- **Energy-level (Grotrian) diagrams** ([#203](https://github.com/shim9610/figgy/issues/203)).
  `CONFIG.md` (both languages) gains the `EnergyLevels` row and section,
  `rich_label`, `category_groups` and `plot_background`, and `SCHEMA.md`
  (in Korean) their JSON form.

<a id="release-0181"></a>

## crates.io patch — renderer 0.18.1 / model 0.12.1 / figgy 0.15.1 (2026-10-21)

Selection boxes, hit tests and picks now come from the same functions that
draw the element, instead of a separate estimate
([#37](https://github.com/shim9610/figgy/issues/37)). Logarithmic axes draw
large values at their height on Metal, DX12 and browsers. No public API is
removed or changed in signature; the additions are listed at the end. No
serialized field changes: documents written by model 0.12.0 parse and
re-serialize byte for byte. The WASM integration example advances to 0.15.1
with the renderer dependency and is not published (`publish = false`).

- **Tick labels.** For axis and colourbar tick labels, `bounds` is the union
  of the labels the raster draws, and the hit region is the selection box
  around them. Before, both came from a representative strip: a sample-width
  band across the axis, and the data area (or the bar) along it. End labels
  hung outside it, wide labels left it, and an empty band beside narrow
  labels was selectable. Polar angle and radius labels hit on each drawn
  label, where their outline is drawn.
- **Tick-label drag.** Dragging labels along their axis moves the selection
  box and the hit region with them again. Since 17d5d9b (2026-08-17) the
  strip dropped `label_offset_x` on top and bottom labels and
  `label_offset_y` on left and right labels. Overlay and subplot axis labels
  share the fix.
- **Axis lines and ticks.** A Cartesian or colourbar axis is selected inside
  the box around its drawn line and ticks. The polar radius ray hits on its
  line and every tick, instead of a fixed 4 px strip that cut longer ticks
  off.
- **Axis titles in layered documents.** Axis titles are placed on the
  expanded base panel the renderer draws, whose right margin follows the
  widest band of the group.
- **Legend and colourbar borders.** The legend box and the colourbar strip
  hit their whole border, including its outer half.
- **Annotations.** An item's bounds, hit test and selection box come from
  `annotation_shapes`, the shape list the raster paints:
  - strokes are grabbed across their width, and at least 3 px either side;
  - an arrow hits on its shaft and head only;
  - the labels of reference lines and arrows are part of the item.
- **Bars and box plots.** Picks follow the shader's coverage: the four
  subpixel samples, and the half-open ownership of shared stack edges. Every
  painted pixel picks its bar or box.
- **Output scale.** `Config::scaled` keeps the fixed tick-label gap and
  legend inset proportional by moving labels and legend through their
  offsets. Before, at 2x the bottom labels sat 2 logical px closer to the
  axis than at 1x. Only scaled display and export copies carry the
  correction.
- **WASM.**
  - The bounded wrapper prepares edits, adoption and new documents at the
    surface scale, so hits match the canvas between frames.
  - `decoration_bounds` returns the per-label boxes of polar tick labels as
    `labels`.
- **Data picks.** The GPU pick runs the draw's own shape functions, so a
  pick lands where the series paints:
  - markers use the scatter shader's distance for all 26 shapes, and
    zero-size markers, which draw a dot, pick and get the selection ring;
  - lines pick on the drawn stroke, with its minimum width, square caps and
    dash gaps;
  - sketch strokes and markers pick on what their style draws, and
    Milkyway and Constellation markers on their documented envelope;
  - point and grid vectors pick on the drawn arrow, and arrows that are not
    drawn do not pick;
  - the selection ring sits `radius_extra_px` outside the drawn marker, at
    every display scale;
  - subpixel histogram bins pick and outline as the envelope paints them,
    and the envelope fills from `bar.baseline`;
  - heatmap cells pick at the painted pixel centre and only where the fill
    is visible;
  - contour label gaps neither pick nor highlight.
- **Subplot selections.** A picked point, bin or cell of a subplot or
  subplot overlay series is highlighted. Expansion never handed a subplot
  the document's `picked_points` or `picked_data`, so no panel drew them.
- **Polar radius labels.** The ray's labels are thinned where their drawn
  boxes collide. They were thinned by their height on screen, as on a
  Cartesian left axis, so rotating the ray toward horizontal dropped all but
  one label.
- **Logarithmic axes on Metal, DX12 and browsers.** Values past about
  2^24 (1.7e7) were drawn at the wrong height, and from about 1e8 at the
  axis floor or not at all; smaller values were off by up to half a decade.
  The shaders chose between the linear and the logarithmic value with
  `mix`, which those drivers evaluate as `x + a * (y - x)`, so a large
  value swallowed its logarithm. A logarithmic axis now takes the
  logarithm directly, for the draw, the pick, the arrow slope and the
  logarithmic colour ramp; the linear arithmetic is unchanged.
- **Polar dash phase.** A dashed or sketched line on a polar chart restarts
  its dash phase when only the polar geometry changes (the inner radius,
  the sector, the zero angle or the direction). The arc-length cache key
  left the transform's polar words out and reused the previous lengths.
- **Pick compile time.** The pick shader is assembled from fragments, and
  only the exact variant a series needs is compiled, on its first pick.
  Enabling GPU picking compiles only the gate pipelines.
- **Tests.** Each element above has a test that compares its drawn pixels
  with its hit region and selection box. `pick_draw_agreement.rs` does the
  same for every data family at 1x and 2x. `scale_consistency.rs` checks
  bounds, label sets and box-filtered images at 1x, 2x and 3x.

Additions:

- the provided `MeasureText::drawn_parts` method. Its default returns
  `false`, so other implementations compile unchanged. The representative
  strip is removed: under a measure that does not lay tick labels and axes
  out, they have no bounds, because only the renderer knows where they are
  drawn. Every host in this repository uses the renderer's `CpuTextMeasure`;
  a custom `MeasureText` keeps axis and tick-label selection by forwarding
  `drawn_parts` to a `CpuTextMeasure`;
- `layout::DrawnPart`;
- `annotation::AnnotationShape`, `annotation::annotation_shapes` and
  `AnnotationElement::shapes`;
- `layout::LEGEND_BORDER_WIDTH`.

Left for [#38](https://github.com/shim9610/figgy/issues/38), because each
needs a public type to change:

- per-part hit regions of multi-part bounded decorations;
- masking a lifted radial label to its slice;
- drag containment measured on drawn bounds;
- annotation label gaps proportional to the Cartesian display scale.

Still approximate after this release: a selected grid-vector cell is
outlined as a whole cell; band fills pick by opacity only; histogram columns
with mixed signs may pick a bin the envelope does not paint; streamed
sources keep their existing pick behaviour.

<a id="release-0180"></a>

## crates.io release — renderer 0.18.0 / model 0.12.0 / figgy 0.15.0 (2026-10-20)

Renderer 0.18.0 and model 0.12.0 are new 0.x minor versions on crates.io. Cartesian charts
gain polar coordinates, vector fields, point colour and size maps, area fills
and step lines, secondary Y axes on overlay layers, and subplots. A radar
family joins the radial, categorical and box plot families, every chart
family gains an annotation layer, and the radial, categorical and box plot
charts gain independently styled titles. Automatic contour labels sit only on
crossed levels, and rotated label backgrounds are antialiased. New public
fields and enum variants break exhaustive struct literals and matches, and
some low-level `data_render` signatures change: see
[Migrating from 0.17.0](#migrating-from-0170). Documents written by model
0.11.0 and 0.10.0 parse and re-serialize byte for byte. The WASM integration
example advances to 0.15.0 with the renderer dependency and is not published
(`publish = false`).

### Polar coordinates

- `Config.polar: Option<PolarOptions>` (#26) draws a Cartesian document in
  polar coordinates: `bottom_x` is the angle axis, `left_y` the radius axis,
  and scatter and line series (with fills and steps), columns, picking and
  export reuse the rectangular paths. `PolarOptions` carries the zero
  angle, the direction, the angle unit (`Degrees` / `Radians`), an optional
  sector, an inner radius fraction, the segment interpolation (`Chord`, or
  `Arc { pieces }` subdividing each segment in data space so dashes and
  picking walk the same pieces) and the radial label angle. `None` (key
  absent) is unchanged. The angle is the x value in the angle unit,
  measured from the zero angle, so the circle is always one full turn:
  the `bottom_x` range (`set_x_range`) chooses only which angle ticks,
  spokes and labels are drawn, and `sector` shows part of the circle.
- The shared `Transform` block grows from 112 to 144 bytes with the polar
  constants (`polar_angle`, `polar_extent`); `data_to_ndc` branches to
  `polar_ndc` and `sample_valid` masks a radius below the axis start
  (`SHADER_COMMON.md` §1 and §4, every synced
  shader, `ScatterTransform`, the pick shader and the CPU twin
  `polar_projection`). The line shaders' `step_piece` helpers take a mode
  word whose upper bits are the arc pieces; the area fill's baseline
  projects through the same mapping. The vertex entries of the scatter and
  line shaders hand the fragment stage polar clip varyings, so a primitive
  past the outer ring (a zoomed-in radius axis), inside the hole or across a
  sector edge is cut there, as the data-area scissor cuts a rectangular
  chart.
- The grid draws rings and spokes, the angle axis is the outer ring with
  its labels around it and the radius axis the ray at the radial label
  angle; new `LabelFormat::Degrees` and `LabelFormat::Radians`. Dragging
  follows the polar frames: the ray turns around the centre
  (`radial_label_angle_degrees`), the angle labels move along their spokes
  (`bottom_x` `label_offset_y`, outward positive) and the radius labels in
  the ray frame (`left_y` `label_offset_x` along, `label_offset_y` across);
  the ring and the top and right axes register nothing. A selection of the
  angle or radius tick labels outlines each label where it is drawn, not one
  box around the whole circle: `SelectionBox` gains `outline`
  (`SelectionOutline::Rect`, or `TickLabels(side)`, which
  `AxisLabelElement` reports in polar mode), and the renderer boxes every
  label of that side from the same layout the axes draw. New
  `Draggable::drag_from` and `Config::nudge_from` carry the pointer
  position these rules read (`DragConfig::nudge_at` for state types); the
  defaults fall back to `drag_by` / `nudge`, and `FiggyChart::on_move`
  sends the position. `Config::polar_geometry` returns the shared polar
  circle (`PolarGeometry`: centre, outer radius and hole radius in pixels).
  Annotations draw only ratio-anchored text and arrows; data anchors,
  reference lines and spans are skipped with `AnnotationSkip::NoValueAxis`.
  CPU and GPU picking project each sample through the same polar mapping as
  the draw.
- Histograms, fields, contours, error bars, vectors, `overlays`, a
  logarithmic or inverted angle axis, a step line under `Arc` interpolation
  and streamed sources are explicit errors in polar mode; polar and
  rectangular subplots mix in a document.
- New example `polar_plot.rs`, a playground scene and `CONFIG.md`
  ("`PolarOptions`").

### Radar charts

- New bounded family `RadarRenderer` / `RadarChart` (#26). A
  `RadarChart` holds 3 to 64 `categories` (one spoke each), 1 to 16
  `RadarSeries` with one value per category, a shared value axis
  (`value_range`, `ring_spacing`; `None` fits a nice axis over the data,
  starting at zero unless a value is negative), `RadarStyle` (start angle,
  direction, polygon or circular grid, ring and spoke colours, line width,
  fill opacity, marker shape and size, emphasis brightness), an optional
  per-series `RadarSeriesStyle`, data `labels` (`None`, `Values`,
  `Emphasized`), `hovered` / `selected` `RadarTarget`s and
  the common `decorations`, `chart_title`, `active_layer` and `annotations`
  fields. A non-finite value breaks the outline at its spoke and drops the
  series' fill and marker there. Per-spoke normalization is the host's job.
- The renderer draws fills, outline segments and vertex markers as signed
  distances in its own `radar.wgsl` pipeline over a CPU-rasterized grid, with
  the bounded frame cache, label-raster reuse across hover and selection,
  `hit_test` returning `RadarPick { target, part }` (`Marker` names a vertex,
  `Outline` and `Fill` the series), the shared decoration map (new
  `DecorationId::SpokeLabel { category_id }` for the category labels, the ring
  labels as `AxisLabel(Left)`), the annotation layer with ratio anchors, PNG
  export at scales from 0.5 to 4 (on a canvas of at least 240 × 240 logical
  pixels), and the `FiggyBoundedChart` web host (`"radar"` family;
  `step_selection` walks series, or a selected vertex's series' vertices).
  Playground scene, `radar_gallery` and `radar_editor` examples,
  `RADAR.md`.
- The value axis is `value_range` / `ring_spacing`, not a full
  `AxisOptions`; markers are `None`, `Circle` or `Square`; a `RadarTarget`
  names a vertex through its optional `category_id` and the whole series
  without it. JSON without the family is unchanged.

### Vector fields

- New `DataRenderType::Vector { components, style }` draws an arrow per sample
  (#25). `VectorComponents::Points { u, v }` reads one (u, v) column pair
  index-aligned with the series' x / y anchors; `VectorComponents::Grid { u, v }`
  reads two matrices declared like a heatmap's (`MatrixRef`, same orientation
  and layout) over the series' coordinate columns, anchored on the coordinate
  under `Centers` and on the cell centre under `Edges`. `VectorStyle` carries
  the colour, shaft width, head length and width, a `min_length_px` below
  which an arrow is hidden, a `pivot` (`Tail`, `Middle`, `Tip`), the scale
  (`VectorScale::Pixels { px_per_unit }`: a fixed pixel factor, the direction
  following the axes' pixel-per-unit ratio; `VectorScale::Data { factor }`:
  the tip at `anchor + factor * (u, v)` in data space, so the arrow follows an
  inverted or logarithmic axis) and an optional `color_map` through
  `Config.colorbar` (`VectorColorMap::Column` for points, `::Grid` for grids).
  `VectorStyle::fit_scale` writes the scale from host statistics, like
  `PointSizeMap::fit_domain`.
- The shader is assembled in Rust from WGSL fragments with fixed interfaces
  (Cartesian projection, input form, arrow glyph, entry); the glyph works on
  an already projected tail and tip and does not depend on the projection.
  The shader compilation test compiles every assembled module. The arrow is
  six vertices per instance straight off the GPU column pool, with analytic
  coverage; no per-arrow value is stored. Common blocks:
  `SHADER_COMMON.md` §5.3.
- Point vectors pick, ring, hover and step through the existing point paths
  with the glyph's radius (half the wider of head and shaft); grid vectors
  pick and highlight by cell on their `u` grid, like a heatmap cell. Auto fit
  includes the tips of data-scaled point vectors (`GpuSeriesExtentMode::VectorTips`);
  pixel-scaled vectors and grid vectors fit their anchors, so a grid's
  data-scaled tips are not part of the fit. Streamed sources are rejected
  with an explicit error; stylized draw styles (Sketch, Milkyway,
  Constellation) skip vector series, and `draw_style_supports` returns
  `false` for them. The legend symbol is a rule with an arrowhead. The web
  `selection_details` reports `u`, `v` and, when colour mapped,
  `color_value` for both forms. The web `auto_fit_colorbar` also fits
  vector colour maps: a point vector's colour column and every column of a
  grid vector's colour grid.
- JSON without the variant is unchanged. New example `vector_field.rs`;
  guide text in `CONFIG.md` ("Vector fields") and `web/SCHEMA.md`. New in
  the low-level `data_render` module: the `vector` module
  (`ColumnVectorLayer`, drawn through `SeriesLayers::vector`), and
  `FieldCellTables`, `create_field_cell_tables`,
  `create_field_cell_bind_group` and `FieldParamsGpu::cell_geometry`, which
  bind a grid vector's cell geometry for its cell pick and selected-cell
  outline.

### Point colour and size maps

- Scatter series (every marker-drawing `DataRenderType`) gain two optional
  value maps (#24). `DataScatterStyleConfig.point_color_map` colours each
  point by a value column through `Config.colorbar`, the same range, ramp and
  `nan_color` a heatmap uses, so the chart keeps one z scale; a colour-mapped
  series without a colourbar is rejected. `point_size_map` sizes each point by
  a value column over an explicit `domain_min..domain_max` onto
  `min_px..max_px`, by marker area (default) or radius; values outside the
  domain clamp, a non-finite value hides the point, and the domain is never
  inferred from the column (`PointSizeMap::fit_domain` writes it from host
  statistics). Precedence per channel is override > value map > style table >
  base.
- The shader reads the value columns from the GPU column pool on every draw
  and derives colour and radius in place; nothing per point is stored. The
  mapped scatter and pick-ring pipelines gain a group-3 bind group with the
  colourbar's parameters and ramp and two value-column vertex slots. The
  group holds `data_render::PointValueMapParamsGpu`, built by
  `create_point_value_map_bind_group_layout` and
  `create_point_value_map_bind_group` (`PointValueMapParamsGpu::NEUTRAL` for
  a series without maps); it is cached per series and rebuilt only when the
  colourbar or the size map changes.
- `pick_point` (CPU and GPU), the picked ring, PNG export at every export
  scale (0.25 to 8) and `selection_details` (new `color_value` /
  `size_value` fields) follow the mapped radius and colour.
  `Chart::auto_fit_colorbar_union` fits the column ids it is given, so a
  Rust host adds the colour-map columns to that list; the web
  `auto_fit_colorbar` includes them automatically. Streamed sources are
  rejected with an explicit error; stylized draw styles ignore the maps like
  the style table.
- JSON without the keys is unchanged. New example `value_maps.rs`; guide
  text in `CONFIG.md` ("Point colour and size maps") and `web/SCHEMA.md`.

### Area fills and step lines

- Cartesian line series can fill the area under the line or between two
  lines, and connect their samples with steps. `DataLineStyleConfig` gains
  `fill: Option<LineFill>` (`ToBaseline(f64)`, default `0.0`, or
  `ToSeries(SeriesRef)` for an index-aligned fill between this line and
  another series that shares the same x column), `fill_color: Option<Color>`
  (`None` derives the line colour at `DEFAULT_FILL_ALPHA` = 0.25 alpha; see
  `effective_fill_color`) and `interpolation: LineInterpolation`
  (`Linear` default, `StepPre`, `StepPost`, `StepMid` with the riser at the
  data-space midpoint). All three are `serde(default)` and omitted from JSON
  at their defaults, so existing documents parse and serialize
  byte-identically.
- The data pass gains a fill loop: every series' fill is drawn before every
  series' body and every picked highlight, each loop in SSOT series order
  with no automatic sorting, so another series' fill can never cover this
  series' line and fills composite in declaration order. The fill is a
  property of the line, not a hit target. The line shader gains the
  `vs_area` / `fs_area` entries on their own pipeline ("figgy line area
  pipeline"); the `Style` uniform keeps its 80 bytes, with the step mode in
  `primitive_flags` bits 2-3 and the fill-to-series bit 4 (SHADER_COMMON.md
  §2.2). Step pieces are drawn by the same line pipelines with a larger
  per-instance strip; the arc-length scan (`line_arc.wgsl`, `ArcParams.mode`)
  and the CPU and GPU pickers walk the same pieces, so dashes keep their
  phase through corners and a pick on a riser resolves to the nearer source
  index by position along the path. The streaming arc replay's parameter
  uniforms grow to 32 bytes.
- Constraints: fills assume x-monotonic series (a doubled-back series
  blends its overlapping trapezoids twice); fill between two series requires
  the same x column and rejects a different column, an unknown series or a
  self reference with `InvalidSeriesConfig`; both fills and step modes
  require GPU-resident columns and are rejected on streamed sources by
  `inspect_streaming_capability` and the streaming prepare path; a gap in the
  line breaks the fill where it breaks the line; a nonpositive baseline on a
  logarithmic y axis is clamped to the axis minimum instead of masking the
  area (an explicit exception to the nonpositive-sample rule).
- The legend mark of a filled series shows a band of the fill colour under
  its rule inside the fixed-width symbol field: `RichSegment` gains
  `rule_band: Option<Color>` (`serde(default)`, omitted when `None`) and
  `series_symbol_segments` emits it. Editing `fill`, `fill_color` or
  `interpolation` moves the data revision like any other `render_type`
  change, leaves the view and selection revisions alone, reuses column
  allocations and refreshes the first frame to a fresh renderer's output.
  The GPU picker's query uniform gains the step mode (320 bytes with the
  point size map and polar fields); the pick registry signature includes it,
  so a step edit rebuilds only that series' pick slot. `DataLineStyleConfig`
  implements `Default`.
- New in the low-level `data_render` module: `ColumnAreaLayer` (the fill
  layer in `SeriesLayers::area`), `line_strip_vertex_count(per_piece,
  interpolation)` and `LINE_PRECISE_VERTICES_PER_INSTANCE`. See the
  configuration reference ("Area fill and step lines") and the interaction
  guide.

### Overlay layers

- Layer addressing, shared by overlay layers, subplots and the annotation
  layer: `Config`, `RadialChart`, `CategoricalChart` and `BoxPlotChart` gain
  `active_layer: LayerRef`, the layer that receives pointer input: `Base`
  (default), `Overlay(i)`, `Annotations`, `Subplot(i)` or
  `SubplotOverlay(i, j)` (see Subplots). The field is `serde(default)` and
  omitted from JSON at `Base`, so existing documents parse and serialize
  byte-identically. `DecorationId` gains `Overlay { layer, inner }`
  (`overlay_<n>:<inner id>`), which `HitMap::for_active_layer` registers for
  the active overlay.
- `Config` gains `overlays: Vec<OverlayLayer>` (#11):
  secondary Y axes drawn on the right side of the shared data area, each
  scaling the series it lists. `OverlayLayer { y_axis, series, legend }` holds
  only what may differ per layer; the chart area, both X axes, the title, the
  grid, the merged legend, the colourbar and `draw_style` exist once on the
  parent. The field is `serde(default)` and omitted when empty, so existing
  documents parse and serialize byte-identically. `Config::expand()` is the
  one expansion path into internal single-layer panels that share the group
  data area (`Expansion { panels, data_area }`); `Config::margins()` now
  reports the group right band, `fit_to_data_area` / the data-area drag write
  it through the parent axis, and `Config::scaled` scales overlay axes and
  legends. `Config::layer_of_series`, `validate_overlays`,
  `active_layer()` (out-of-range fallback) and `set_active_layer` complete the
  model API.
- Interaction target layer: `Renderer::set_active_layer` / `active_layer`
  (also on `WindowedRenderer`) store the layer, clear the element selection
  on a change and recover an out-of-range index to `Base` through every
  registry entry point. `HitMap::for_active_layer(cfg)` registers the shared
  elements first and the active overlay's `OverlayAxisElement` /
  `OverlayAxisLabelElement` / `OverlayAxisTitleElement` /
  `OverlayLegendElement` last (`overlay_<n>:…` ids); their drags write back
  into `overlays[n]` only. GPU picking addresses the active layer's panel and
  series.
- Rendering: `ChartView` holds one panel per layer (`overlay_count`,
  `layer_raster_generations`), `create_chart_view` / `refresh_axis*` keep the
  panel set in step with the document and re-rasterize only the panels whose
  internal config changed, so a layer edit leaves the other layers' textures
  intact and the cached view draws byte-identically to a fresh render.
  `prepare`, `prepare_registered` and export expand a layered item into one
  draw item per panel; picked highlights keep item order. Render warnings are
  merged with `WarningAxis::OverlayY(index)`. `ChartViewState::overlay_y`
  carries one Y range per overlay so one linked range edit moves every layer
  atomically. `Series` is now `Copy`.
- Export: `export_document_rgba_async` / `export_document_png_bytes_async`
  (with `_with_clear` and blocking native variants) render the whole document;
  `export_panel_*` stay and render the base panel. New native example
  `overlay_axes`.
- Fitting: `Chart::set_overlay_y_range(layer, min, max)` and
  `Chart::auto_fit_overlay_y_extent` / `auto_fit_overlay_y_union` write
  `overlays[layer].y_axis`.
- Limits: overlays own no X axis, have no overlays of their own, exclude
  bounded charts, and a nonresident streaming series listed by an overlay is
  rejected (`InvalidSeriesConfig`); the stream request paths reject a
  document with overlays (`InvalidConfig { field: "overlays" }`).
- Revisions: the registry's revisions follow a single exhaustive
  classification of `Config` fields instead of refreshing the axis raster on
  every config replacement. `set_chart_config` and `set_chart_state` leave
  every revision unchanged for an identical config, bump `desired` and
  `config` for any change, bump `raster` only for chrome fields
  (`raster_config_equal`: chart area, axes, title, grid, legend, colourbar,
  draw style, plus any series change), `view` for data geometry
  (`stream_config_equal`) and `selection` for `picked_*`. The SSOT
  invalidation matrix asserts those revisions and the pixel oracle for every
  `Config` field, now including `chart_area.x` / `y`, `label_font`,
  `significant_digits`, the Milkyway and Constellation styles and every
  colourbar field, behind an exhaustive destructuring guard so a new field
  cannot land unclassified.
- See the interaction guide, the configuration reference and the WASM guide.

### Subplots

- Subplots (#11): `Config` gains `subplots: Vec<Subplot>`, more charts
  drawn in the same document. A `Subplot { config, series,
  link_x, link_y }` is a complete Cartesian chart with its own `chart_area`
  (in the parent's surface coordinates), all four axes, title, grid, legend,
  colourbar, style and secondary Y axes (`config.overlays`); the document's own
  `Config` stays the first chart, so a 2×2 grid is the parent plus three
  subplots. Nothing is shared unless linked: `link_x` / `link_y` (`CellRef::
  Parent` or an earlier `CellRef::Subplot(j)`) make `bottom_x` / `left_y`
  follow the target's range, scale, direction and tick spacing while the
  axis style stays the subplot's own. The field is `serde(default)` and
  omitted when empty, so existing documents parse and serialize
  byte-identically.
- One value per link group: `Config::subplot_config(i)` resolves a subplot
  with its links applied, `Config::sync_subplot_links(previous)` carries an
  edit of any linked axis to the root of its group and stores every member
  equal to it, and `Config::subplot_link_root` names the root. The registry
  runs it inside `register_chart`, `set_chart_config`, `set_chart_state` and
  `set_chart_view_state`, so a linked edit moves every member in one
  `&mut self` call. `ChartViewState::subplots` carries one view state per
  subplot. `Chart::set_subplot_x_range` / `set_subplot_y_range` and
  `auto_fit_subplot_{x,y}_{extent,union}` write the root of a linked axis.
  `Config::validate_subplots` rejects nested subplots, an annotation layer on
  a subplot, a link to a later subplot and a series id drawn twice.
- Rendering: `Config::expand` appends each subplot's panels after the
  parent's (`Config::panel_layers` gives the order), each in its own area with
  its own data area, raster cache and transform, so an edit of one chart
  re-rasterizes that chart's panels only and the cached frame is
  byte-identical to a fresh render. Each selection box goes to the topmost
  panel of the chart it sits in. The document covers the union of every chart
  area (`Config::document_area`, `Config::translate_areas`):
  `display_config_for_surface` fits it and returns where the parent lands (new
  `place_document`), `GpuPickRequest::display_panel_px` is the displayed
  document rect, `export_document_*` renders the union and `export_panel_*`
  keeps the parent's area. The annotation layer stays in the parent's area and
  its pointer route answers there only while `Annotations` is active. Subplot
  warnings are tagged `WarningAxis::Subplot { index, axis: SubplotAxis }`.
  Streaming rejects a document with subplots and a streamed series drawn by a
  subplot, as it does for overlays.
- Interaction: `LayerRef::Subplot(i)` addresses a subplot and
  `LayerRef::SubplotOverlay(i, j)` one of its overlays. An out-of-range
  `Subplot(i)` reads as `Base`; `SubplotOverlay(i, j)` reads as `Subplot(i)`
  when `j` is past that subplot's overlays and as `Base` when `i` is out of
  range. While one is active `HitMap::for_active_layer` registers exactly
  that subplot's own set, wrapped as `SubplotElement`s (`subplot_<i>:…` ids);
  drags and resizes use the single-chart policies on the subplot's chart and
  land in `subplots[i].config`. New native example `subplots`, which writes
  `gallery-subplots.png` (four sensors in a 2×2 grid whose three subplots
  link their X axis to the parent) and `mixed-axes.png` (one linked X axis, a
  chart with a different X column and a twin Y axis from an overlay inside a
  subplot).
- Fitting a whole link group: `Chart::auto_fit_layer_union(pool, layer,
  series, padding)` fits the axes `layer` owns to every series drawn against
  them, so a linked axis covers the data of every chart in its group, and
  `Chart::auto_fit_layer_extent(layer, x, y, padding)` does the same from
  exact extents. `Config::series_fit_roles(layer, series_id)` names whether a
  series counts towards that layer's X and Y fit (X: every chart of the X
  link group, overlays included; Y: the base series of the Y link group; an
  overlay layer: its own series only). The WASM `auto_fit_all` now uses
  both instead of its private copy of the rule, so its results are
  unchanged. The playground's subplot rows offer "fit" (that chart's series)
  and "fit group" (the link group).
- See the configuration reference (`Subplot`), the interaction guide
  (subplots) and the WASM guide.

### Annotation layer

- Every chart family has a dedicated annotation layer (#12). `Config`,
  `RadialChart`, `CategoricalChart` and `BoxPlotChart` gain
  `annotations: AnnotationLayer { visible, items }`, and `RadarChart` has the
  same field. `items: Vec<Annotation>` holds the annotations in draw order;
  an `Annotation` is an `id`, a `visible` flag and one of
  `ReferenceLine { axis, value, style, label }`,
  `Span { axis, from, to, fill }`, `Text { anchor, text, box_style }` or
  `Arrow { from, to, style, text }`, with `Anchor::Data` (axis coordinates that
  follow zoom and range changes) or `Anchor::Ratio` (plot-area fractions, fixed
  on screen). `AnnotationAxis::X` / `Y` are screen axes in every family:
  `bottom_x` / `left_y` on Cartesian charts, the category index axis and the
  value axis on bar and box charts, no value axes on radial and radar charts
  or on a Cartesian chart in polar mode, where reference lines, spans and
  data anchors are skipped with `NoValueAxis`. Non-finite values and
  unplaceable anchors are not errors: the raster skips the item and reports
  `RenderWarning::AnnotationSkipped { index, reason }`
  (`AnnotationSkip::{NonFinite, NoValueAxis, OutOfRange, EmptyText}`).
  Malformed styles are rejected by `AnnotationLayer::validate`, part of
  config validation. Annotations never change the data-area margins and are
  not nudged by the label placer. The JSON shape is additive: `annotations`
  is `serde(default)` and omitted at its default (visible, no items), and an
  empty `items` is omitted, so existing documents parse and serialize
  byte-identically.
- `DecorationId` gains `Annotation(index)` (`annotation_<n>`), which the
  annotation map registers. The chart's own frame depends on neither
  `annotations` nor `active_layer`: annotation items draw into separate
  textures, and the bounded frame and label raster caches exclude both
  fields from their keys, so editing them reuses the cached frame.
- One shared CPU raster pass draws the layer into two panel textures composited
  with the existing fullscreen textured pipeline: the underlay (spans) below
  the data and the overlay above every decoration. Cartesian panels draw
  grid → underlay → data → picked highlight → decoration → overlay, on the
  resident path, in stream displays and in `export_panel_*` (at the export
  scale). `ChartView` carries the textures, `ChartView::annotation_map()` and
  `ChartView::layer_stamps()`; `annotations.items` has its own `annotation`
  revision (`ChartRenderStamp::needs_annotation_raster_since`), answered by
  `Renderer::refresh_annotations(view, chart, panel_rect, selected)`, which
  rewrites the two annotation textures and the annotation hit map only.
  `refresh_axis*` redoes them with the axis raster; `annotations.visible`
  bumps no raster revision and only skips the composites. Bounded renderers
  return the layer as `prepare_annotations(chart, size, scale)` →
  `Arc<BoundedAnnotationFrame>` (`draw_underlay`, `draw_overlay`,
  `annotations()`, `render_warnings()`, `shares_rasters_with`), cached beside
  the frame so an annotation edit or an annotation selection reuses the chart
  frame and its label raster. The new `FrameBackground` option
  (`set_background`, default `Opaque`) lets a frame leave its background to
  the host clear so an underlay shows below bars and boxes; frames are cached
  per mode and `export_rgba` picks the mode itself. `Frame::annotation_space()`
  / `pixel_space_for_config` expose each family's anchor space (`PixelSpace`,
  the `AnnotationSpace` trait). New `raster::Canvas::fill_polygon` fills a
  closed polygon (non-zero winding).
- Every annotation has the standard interaction: `AnnotationLayer` implements
  `DragConfig` and `AnnotationElement` the `Selectable` / `Draggable` /
  `Resizable` contracts, exposed as an `AnnotationMap` (the `DecorationMap`
  shape). Items drag as a whole; a span's edges, a reference line's value and
  an arrow's endpoints have their own selection-box handles under the compass
  names; out-of-range moves are rejected atomically and anchors keep their
  kind (`Anchor::converted` switches kinds in place). Bounded charts select an
  annotation through `decorations.selected = Annotation(index)` without
  rebuilding the frame; Cartesian hosts pass the index to
  `refresh_annotations`.
- Pointer route by the active layer (#12): while
  `active_layer == LayerRef::Annotations` and the layer is shown, the drawn
  items sit on top of the chart's own hit map; on every other layer they are
  not part of the route and clicks pass through them.
  `ChartView::hit_test_active_layer(config, hit_map, point)` tests the view's
  annotation map above `HitMap::for_active_layer` (whose `Annotations` set is
  the shared elements, for single-layer charts and layered documents alike),
  `BoundedAnnotationFrame::hit_test_active_layer(active, decorations, point)`
  tests the items above every decoration of a bounded frame, and
  `AnnotationMap::route` / `receives_pointer` state the shared rule; both
  return `LayerHit::{Annotation { index, id }, Element(id)}`. `RadialChart`,
  `CategoricalChart` and `BoxPlotChart` gain `active_layer()` /
  `set_active_layer` (an `Overlay`, `Subplot` or `SubplotOverlay` reads and
  is stored as `Base`; a change clears `decorations.selected` and keeps the
  data selection), and `LayerRef::bounded()`. A layer switch reuses the
  data, grid and decoration textures and the bounded frames; routed
  selection and drags refresh the annotation layer only, and the cached view
  then draws byte-identically to a fresh render.
- Guides: configuration reference, interaction guide (annotation layer),
  architecture (revision table), bounded family guides, WASM guide and JSON
  schema.

### Independent bounded titles

- `RadialChart`, `CategoricalChart` and `BoxPlotChart` gain optional
  `chart_title: Option<BoundedTitleOptions>` with independent visibility and
  `RichText` (font, size, color, segment styles and `TextAlign`). Legacy JSON
  without the field still uses the plain `title` and shared label style;
  `resolved_title()` returns the title as drawn, from `chart_title` or from
  the legacy fields. Editors call `normalize_title()` once before editing
  labels to preserve the original title appearance. Title hit bounds,
  dragging and export use the same renderer metrics at every output scale.
  Label collision handling is unchanged.
- `validate()` of the three charts also checks the resolved title
  (`BoundedTitleOptions::validate`) and the annotation layer; see
  [Migrating from 0.17.0](#migrating-from-0170).
- See the interaction guide and the `bounded_titles` example.

### Contour label fixes

- Automatic contour labels are placed only where their level is actually
  crossed (#19). The anchor pass used to accept any projected seed whose value
  was within its tolerance of the level, so in a near-flat region a shallow
  extremum just short of a level (for example a minimum of +0.0004 against
  level 0) received a label with no isoline anywhere near it. A seed is now
  kept only if the level lies within the corner range of the cell it lands
  in, the same condition under which the stroke draws a line there. Labels on
  real isolines are unchanged.
- Rotated contour label backgrounds (`bg_color`) are antialiased (#20). The
  label quad ended exactly at the edge of its baked background box, so the box
  edge was the rasterized quad edge and stair-stepped at any non-axis-aligned
  angle while the glyphs inside stayed smooth. The quad now extends two pixels
  past its content into the atlas cell's transparent gutter, continuing the
  same content-to-texel map, so the texture filter fades the box edge over one
  texel. Interior sampling, the label size, the contour line gap and labels
  without a background are unchanged; axis-aligned box edges gain the same
  one-texel antialiasing.
- No API, model or serialization change.

### Heatmap legend swatch

- A heatmap's legend entry shows a swatch of the chart's colour map (six
  bands sampled from `colorbar.colormap`) instead of an empty symbol field.
  `series_symbol_segments` and `update_legend_symbols_preserving_text` take
  the colour map as a trailing `Option<&ColorMap>`; a heatmap without one
  keeps the empty field. The swatch sits inside the fixed-width symbol field,
  so legend hit bounds do not change. The web wrapper refreshes
  auto-managed legend symbols when `set_config` changes the colour map.

### Resident data cache

- A resident Precise panel keeps its series bodies in a panel-sized texture
  and draws them again only when the data layer changes: the transform, the
  data area, the draw style, `polar` and `colorbar`, the series configurations
  and style scales, the font generation, the allocation epochs of the columns
  that panel reads, or the pool layout or pipeline generation. A repaint with
  nothing changed, and every edit or selection of titles, legend, axes, grid,
  annotations or points, blits the cached bodies instead of drawing every
  point again. Picked and selected highlights draw live above them, and each
  panel of a layered document has its own entry, so editing one layer, or
  replacing a column only another layer reads, draws only that layer. Clipped
  and styled panels draw directly as before. The composite can differ from a
  direct draw by UNORM rounding at partly covered pixels; random edit
  sequences are checked bit for bit against a render of the final state from
  nothing. No API or serialization change; `paint_prepared` reports a token
  whose cached bodies were drawn again after it as stale.

### Default layout

- `default_config()` sets `right_y.out_margin` to 32 (was 8). The last
  bottom tick label is centred on the data area's right edge, so half of it
  lies in the right margin; at the default 18 px font even `10.0` needs about
  17 px, and the old 13 px band (margin plus tick) cut it at the chart edge, on
  screen and in exports. The new band holds a seven-character label.
- `default_colorbar_options()` sets `gap_px` to 8 (was 24). The axis band on
  the bar's side already separates it from the data area, so the wider right
  band would otherwise push a right colourbar 61 px away; it now sits 45 px
  from the data area (37 px before), and top, left and bottom bars sit 16 px
  closer.
- Documents keep their stored values: both fields are always serialized, so
  only configs built from the defaults change. To keep the old geometry, set
  `right_y.out_margin = 8.0` and `gap_px = 24.0`.

### Web example

- No method of the WASM integration example `figgy` 0.15.0 is removed or
  renamed, and no signature changes. New configuration and series keys in
  `get_config` / `set_config` / `get_series` / `set_series` and in the
  bounded document JSON are additive and omitted at their defaults.
- Layers: `FiggyChart` adds `active_layer` / `set_active_layer` (the layer
  as JSON), accepts the subplot layers and rebuilds its hit map when the
  layer, the overlay count or the active subplot's overlay count changes.
  `auto_fit_overlay_y(layer, column, padding)` fits an overlay's Y axis and
  `auto_fit_subplot(index, "x" | "y", columns, padding)` a subplot axis (a
  linked axis moves its link group). `hit_test`, `decoration_bounds`,
  `on_press`, `on_move` and `has_selection` follow the active layer and the
  annotation route (`overlay_<n>:…`, `subplot_<i>:…` and `annotation_<n>`
  ids); on the base layer they answer as before. `export_png` exports the
  whole document, and the canvas fit and pointer mapping fit
  `Config::document_area`; without overlays or subplots the output is
  unchanged.
- `<figgy-chart>` forwards `auto_fit_overlay_y(layer, column, padding)`,
  `set_active_layer(json)` and `active_layer()`; its two layer methods return
  the parsed layer value (`"Base"`, `{ Overlay: 0 }`) instead of the
  kernel's JSON string.
- Annotations: `FiggyChart` and `FiggyBoundedChart` expose `annotations`,
  `hit_annotation`, `select_annotation`, `selected_annotation`,
  `annotation_handle`, `drag_annotation`, `add_annotation`, `set_annotation`,
  `remove_annotation`, `set_annotations_visible`, `pointer_to_anchor` and
  `annotation_warnings`; a bounded chart with a visible span is drawn with the
  host-clear frame and the chart background as the clear colour.
  Their pointer methods follow the route: `FiggyChart.hit_test`, `on_press` /
  `on_move` (handles of the selected item first, then a press selects and
  drags an item) and `decoration_bounds` answer `"annotation_<n>"`, and a
  layer change clears the annotation selection; `has_selection` also counts
  it. `FiggyBoundedChart` gains `set_active_layer` / `active_layer`, and its
  `hit_decoration`, `select_decoration`, `resize_handle` and `drag` take the
  same ids. The index-based annotation methods keep working on every layer.
- `FiggyBoundedChart` accepts the `"radar"` family in `create` /
  `set_document` (see Radar charts).
- Behaviour change: `FiggyChart::auto_fit_all(padding)` and
  `<figgy-chart>.auto_fit_all(padding)` fit only the axes of the active
  layer, instead of the parent's axes from every registered series. On
  `Base`, and on the annotation layer, which draws on the base axes, they
  fit the parent's `bottom_x` and `left_y`; on `Subplot(i)` subplot `i`'s;
  on `Overlay(n)` and `SubplotOverlay(i, n)` that overlay's Y axis only.
  Each axis fits the series drawn against it (`Config::layer_of_series`).
  A linked axis (`link_x`, `link_y`) fits the series of every chart in its
  link group and moves the whole group, whichever member is active; X and
  Y links are followed separately. Overlay series count towards their
  chart's X axis, which they share, and not towards `left_y`. A series of
  an unlinked subplot no longer widens the parent's axes. A document
  without overlays or subplots fits as before. To fit every layer, a host
  calls `set_active_layer` and then `auto_fit_all` for each layer, or calls
  `auto_fit_overlay_y` / `auto_fit_subplot` with explicit columns.
  Streaming is unchanged: streamed documents have no overlays or subplots.
- A base `auto_fit_all` now also moves the subplots whose axes link to the
  parent (`commit_auto_fit_all_if_current` resyncs the links).
- `FiggyChart::step_selection` steps a selected point vector
  (`DataRenderType::Vector` with `VectorComponents::Points`) along its
  series' points, as it steps scatter points.

### Documentation and examples

- Documentation: a [polar guide](crates/renderer/POLAR.md) (input and first
  render, coordinate options, axes, picking, dragging, subplots, export;
  English and Korean), a "Chart types" table and short polar and radar
  snippets in the READMEs, `USAGE.md` sections (polar, and the Cartesian
  series, maps and layers: area fills and steps, point colour and size
  maps, vector fields, overlays, subplots and annotations), and twelve new
  gallery images: the polar antenna pattern and compass rose, radar on a
  polygon and on a circular grid, and eight designed scenes drawn by the
  feature examples `vector_field`, `value_maps`, `area_step`,
  `overlay_axes` and `subplots` (a wind field over a heatmap, surface
  currents, colour-mapped points, bubbles, forecast bands, a step
  schedule, twin axes and small multiples). The Korean README gains the
  concept and draw-style rows. The
  [example index](crates/renderer/examples/README.md) shows every gallery
  image with the function that draws it and the command that regenerates
  it. The `precise_scatter_probe` example is removed; the `sketch_render`
  GPU test checks the same plain scatter output.
- The browser guide gains an English
  [streaming contract](crates/renderer/WASM.md#streaming-contract) beside
  its Korean section 6: supplying sources and running jobs, fitting the
  first stream, view changes, selection and export, GPU caches and memory
  limits, and the support scope. The English links in `ARCHITECTURE.md` and
  the web README lead there.
- Released-document fixtures:
  [`released_documents.rs`](crates/renderer/tests/released_documents.rs)
  parses thirty documents written by the published model 0.11.0 and 0.10.0
  with the current types and requires the same bytes back. They cover the
  default `Config`, a `Config` with every series kind of those releases and
  a colourbar, an edited `Config`, every draw style, series lists covering
  every `DataRenderType` variant of those releases, and the radial,
  categorical and box plot charts at their defaults and edited
  ([fixture notes](crates/renderer/tests/fixtures/released/README.md)).
- `tools/playground` is a standalone interactive bench (winit and egui; not
  a workspace member and not published) with scenes for every chart family
  and for area fills and steps, value maps, vector fields, twin Y axes,
  subplots, annotations, polar and radar.

<a id="migrating-from-0170"></a>

### Known limitations

- macOS / Metal remains experimental. On the Apple Paravirtual adapter, a
  Constellation line drawn by the stream path can differ from its resident
  draw by one 8-bit code value where the line overlaps itself. The stream
  against resident style tests accept that on Metal only (at most one code
  value on at most 32 pixels) until it is investigated on Metal hardware; see
  `ci/STATUS.md`. Other backends compare exactly.

### Migrating from 0.17.0

JSON written by renderer 0.17.0 / model 0.11.0 parses and re-serializes byte
for byte: every new key is optional and omitted at its default. The changes
below affect Rust code, and one of them is a stricter validation of bounded
charts. The renderer re-exports the model, so the model notes apply to
renderer users too. Configs built from `default_config()` get a wider right
margin and a narrower colourbar gap; see [Default layout](#default-layout) for
the values that keep the old geometry.

Model types (figgy-model 0.12.0):

- Exhaustive `Config` literals need `polar: None`, `overlays: Vec::new()`,
  `subplots: Vec::new()`, `active_layer: LayerRef::Base` and
  `annotations: AnnotationLayer::default()`, or end with
  `..default::default_config()` (`model::default::default_config()`,
  re-exported as `renderer::default::default_config()`). `Config` does not
  implement `Default`, so `..Default::default()` does not compile.
- Exhaustive `RadialChart`, `CategoricalChart` and `BoxPlotChart` literals
  need `chart_title: None`; literals using `..Default::default()` remain
  valid.
- Exhaustive `RadialChart`, `CategoricalChart` and `BoxPlotChart` literals
  need `active_layer: LayerRef::Base` and
  `annotations: AnnotationLayer::default()`; literals using
  `..Default::default()` remain valid.
- Exhaustive `DataLineStyleConfig` literals need `fill: None`,
  `fill_color: None` and `interpolation: LineInterpolation::Linear`, or end
  with `..Default::default()` (`DataLineStyleConfig` now implements
  `Default`).
- Exhaustive `DataScatterStyleConfig` literals need `point_color_map: None`
  and `point_size_map: None`.
- Exhaustive `SelectionBox` literals need `outline: SelectionOutline::Rect`
  (the box drawn as before).
- `series_symbol_segments(cfg)` and
  `update_legend_symbols_preserving_text(content, series)` take a trailing
  `colormap: Option<&ColorMap>`. Pass the chart's
  `config.colorbar.as_ref().map(|bar| &bar.colormap)`, or `None` for the
  previous output.
- Exhaustive `RichSegment` literals need `rule_band: None`. `RichSegment`
  does not implement `Default`; `..RichSegment::plain(c)` also works.
- Exhaustive `LabelFormat` matches need `Degrees` and `Radians` arms.
- Exhaustive `DataRenderType` matches need a `Vector { .. }` arm.
- Exhaustive `DecorationId` matches need `SpokeLabel { .. }`,
  `Overlay { .. }` and `Annotation(_)` arms.
- Validation: `RadialChart`, `CategoricalChart` and `BoxPlotChart`
  `validate()` also checks `annotations` and the resolved title
  (`BoundedTitleOptions::validate`). Some documents passed 0.11.0 validation
  and are now rejected: a radial or categorical `font_family` longer than
  160 characters or containing a control character, and a radial `title`
  containing a control character. Shorten the font name and remove the
  control character. JSON parsing is unchanged.

Renderer (figgy-renderer 0.18.0):

- `ChartViewState` literals need `overlay_y: Vec::new()` for a chart without
  overlays, otherwise one `AxisViewState` per entry of `Config::overlays`, as
  `Renderer::chart_view_state` returns them. `ChartViewState` does not
  implement `Default`, and `set_chart_view_state` rejects a count mismatch
  with `InvalidConfig { field: "overlays" }`.
- `ChartViewState` literals need `subplots: Vec::new()` for a chart without
  subplots, otherwise one `ChartViewState` per entry of `Config::subplots`.
  Each of those has empty `subplots` and one `overlay_y` entry per overlay
  of that subplot. `set_chart_view_state` rejects a mismatch with
  `InvalidConfig { field: "subplots" }`.
- `RenderWarning` (`non_exhaustive`) gains `AnnotationSkipped`, and
  `RenderWarning::axis()` returns `Option<WarningAxis>`, which is `None`
  only for `AnnotationSkipped`. Write `if let Some(axis) = warning.axis()`,
  or `.filter_map(RenderWarning::axis)` in place of
  `.map(RenderWarning::axis)`.
- `WarningAxis` (`non_exhaustive`) gains the data-carrying variants
  `OverlayY(usize)` and `Subplot { index, axis: SubplotAxis }`, so numeric
  casts such as `axis as isize` no longer compile. Match on the variants
  (with a `_` arm), or use the `Display` field path.
- `Series` now implements `Clone` and `Copy`. A closure that uses a `Series`
  by value without `move` now borrows it instead of moving it, so a closure
  that outlives the function, such as one it returns, needs `move`.
- Exhaustive `GpuSeriesExtentMode` matches need a `VectorTips` arm; existing
  discriminants are unchanged.
- `set_chart_config` and `set_chart_state` leave every revision unchanged
  for an identical config. Hosts that relied on such a call to force a
  raster refresh should edit the chart or refresh the view directly
  (`refresh_axis`).

Low-level `data_render` module (figgy-renderer 0.18.0):

- `ScatterTransform` literals need `polar_angle: [0.0; 4]` and
  `polar_extent: [0.0; 4]` for a rectangular chart;
  `scatter_transform_from_config` fills them for a polar one.
- `SeriesLayers` literals need `area: None` for a series without a fill.
- `SeriesLayers` literals need `vector: None` for a series that is not a
  vector field.
- `ColumnScatterLayer` and `ColumnPickRingLayer` literals need
  `value_map_bg`, `color_value` and `size_value`. Without value maps write
  `color_value: None, size_value: None`. `value_map_bg: None` is valid only
  together with `style_map_bg: None` (the plain pipeline). A layer drawn by
  the mapped pipeline must bind group 3, and `None` panics at draw time. For
  a series without value maps, pass the neutral group:
  `let (neutral, charge) = create_point_value_map_bind_group(device, &ledger, &value_map_bgl, &PointValueMapParamsGpu::NEUTRAL, &[]);`,
  then `value_map_bg: Some(&neutral)`. Here `ledger` is the
  `Arc<gpu_memory::GpuLedger>` the function takes and `value_map_bgl` is the
  layout given to `create_scatter_columnar_mapped_pipeline`. Keep `charge`
  alive as long as `neutral`.
- `draw_chart_panel_columnar` takes `annotation_underlay: Option<AxisLayer>`
  after `grid` and `annotation_overlay: Option<AxisLayer>` after
  `decoration`. Pass `None` for both to draw as before:
  `draw_chart_panel_columnar(pass, target_size, panel_rect, data_area, grid, None, &series, decoration, None)`.
- `create_scatter_columnar_mapped_pipeline` takes `value_map_bgl` after
  `style_map_bgl`; pass `&create_point_value_map_bind_group_layout(device)`.
  The pipeline now also reads group 3 and vertex slots 4 and 5.
  `draw_chart_panel_columnar` binds them from the layer's `value_map_bg`,
  `color_value` and `size_value` (see the `ColumnScatterLayer` note).
- `line_arc::ArcScratch::build` takes a trailing `step_mode: u32`. Pass `0`
  for a linear line, the previous behaviour. The word holds the step mode in
  bits 0-1 (`1` step-pre, `2` step-post, `3` step-mid) and the polar arc
  pieces from bit 4.

## crates.io release — renderer 0.17.0 / model 0.11.0 / figgy 0.14.0 (2026-10-16)

Renderer 0.17.0 and model 0.11.0 are new 0.x minor versions on crates.io:
`RichText.align` is a new public field, so fully specified `RichText` literals
need the migration noted below. The WASM integration example advances to 0.14.0 with the renderer dependency
and is not published (`publish = false`).

- The raw WASM host adds `FiggyBoundedChart` for radial, categorical and boxplot
  documents, using the existing renderer's geometry, decoration drag/resize,
  hover, typed selection and PNG export. Radial clicks retain the hovered
  slice's resting hit target during lift animation. `FiggyChart` adds measured
  `decoration_bounds` and source-index `step_selection` for host UI parity;
  `HitMap::find` resolves stable element IDs. These APIs do not add automatic
  layout or mutate unrelated SSOT properties.
- `FiggyChart.selection_details` reads selected resident GPU values for points,
  histogram bins and matrix cells without retaining source arrays. Histogram
  reports include bin boundaries, stored value and orientation; keyboard
  navigation follows the orientation and clamps when switching compatible series.
- Line and scatter paths omit non-finite coordinates and non-positive values
  on logarithmic axes without rejecting the remaining data. A line segment
  touching an invalid endpoint leaves a gap; precise, sketch, ribbon/arc and
  star paths agree, and invisible points cannot be picked. Histogram zero
  baselines retain their positive-range clipping behavior.
- Numeric tick precision now respects `LabelStyle.significant_digits` for
  arbitrary auto-fit intervals and logarithmic labels. A step such as 1.312815
  no longer forces six decimal places, avoiding unnecessary title overlap and
  colorbar clipping. Linear decimal labels keep a shared precision with a
  spacing-derived minimum; crowded numeric labels use a regular stride
  without changing ticks, grid positions, margins or title anchors. Contour
  label precision uses the level spacing rather than the bottom X-axis spacing.
- Bounded renderers add `new_async` for checked asynchronous browser pipeline
  compilation; native construction keeps the existing synchronous behavior.
  The bounded web host's `first_frame_ready` waits for submitted GPU work and
  reports a rejected completion instead of announcing an unusable chart.
- `ChartView::render_warnings` reports the tick settings the axis raster
  replaced instead of substituting them silently: a linear `major_spacing` that
  is non-finite, not positive, too small to advance or over 1000 intervals
  (drawn at automatic spacing), a linear `minor_count` over 99, and a log decade
  step that is not a whole number from 1 to `i32::MAX`. Each `RenderWarning`
  names the axis (`WarningAxis`), the value as written and the value drawn; a
  replaced spacing also says why (`SpacingRejection`). `create_chart_view` and
  each successful `refresh_axis` / `refresh_axis_with_selection` re-derive the
  list from the chart they rasterized, so hosts read it in their dirty-driven
  refresh branch and nothing is computed per frame. Additive: no signature
  changes, and the drawn output is unchanged.
- `RichText` gains `align: TextAlign` (`Left` default, `Center`, `Right`;
  re-exported from both crate roots). Each line of a multi-line text is offset
  inside the block by none, half or all of its slack; tab cells move with their
  line, and the measured envelope (selection and hit boxes) is unchanged. JSON
  without `align` parses as `Left`, and `Left` is not serialized. Migration:
  fully specified `RichText` literals need `align: TextAlign::Left`.
- The title helpers no longer clip a multi-line or enlarged chart title.
  `Chart::with_title` and the web example's `set_title` call
  `layout::fit_chart_title_margin`, which grows `chart_title.top_margin` to the
  measured title plus the default band's padding only when the title block is
  taller than the band (exactly the titles that were clipped); every title
  that already fit renders as before. The band is never shrunk. A
  `top_margin` injected directly through the SSOT is kept as given.
- The API guide distinguishes direct SSOT edits from convenience helpers.
  Editing only axis `min` / `max` preserves tick settings; `set_*_range` and
  `auto_fit_*` also choose ticks and label formatting. Title helpers document
  whether they fit the title band or change only text.
- `draw_style_supports(style, render_type)` reports whether a draw style draws
  every primitive of a render type, from the same selection prepare uses.
  Sketch and Milkyway draw line, scatter and errorbar series and skip bars,
  fields and contours; Constellation draws `ScatterLine` only. Prepare skips
  an unsupported series without an error, so hosts can now reject it up front.
- `text_render::font_family_available(family)` reports whether a
  `RichText.font` / `label_font` family resolves to a registered or installed
  face instead of silently falling back to the bundled Liberation Sans. It
  uses resolution's exact, case-sensitive match; the empty string and
  `"Liberation Sans"` are always available, and on wasm only registered fonts
  count. The check itself changes no rendering.
- Characters missing from the selected font no longer draw as `.notdef` boxes
  when another available face can draw them (for example Greek in a Latin-only
  family; U+207B, which the bundled Liberation Sans lacks, comes from an
  installed system font on native hosts that have one). Measurement and
  drawing take each such character from the first face that can draw it:
  fonts added with `register_font_bytes`, family by family in registration
  order with the requested bold/italic matched as in a family lookup; then the
  bundled Liberation Sans of the same style; then, on native targets only,
  installed system fonts in family-name order. Faces without outlines
  (colour-bitmap emoji) do not count, since text rasterizes from outlines.
  Registered and bundled faces resolve identically on native and wasm, and a
  registered font now also supplies characters the configured family lacks
  (for example Hangul in a Latin label font). Glyphs the selected face covers,
  including Comic Neue in sketch mode, render as before; text containing
  previously missing characters measures and draws differently.
- The CPU colourbar strip no longer shows translucent seams when its start or
  length falls on a fractional pixel. Interior band edges now snap to device
  pixels, so no pixel takes two partial anti-aliased covers; a grayscale strip
  previously showed plateaus and jumps at about a third and two thirds of its
  length. Band colours, ramp sampling (translucent stops included) and the
  strip geometry are unchanged.
- The CPU colourbar raster draws only the bands that land on the canvas. A
  strip following a data area that runs far past the chart edges no longer
  costs work proportional to its length (#6: billions of bands on a 320×240
  canvas).
- Negative margins remain coordinates that may move the data area past the
  chart edges. `Config::data_area` now refuses a layout it cannot represent or
  that would leave nothing to grab: a non-finite margin (`Infeasible`), a data
  area starting above or left of the canvas origin or ending past `u32::MAX`,
  and one keeping less than `MIN_VISIBLE_DATA_AREA_PX` (8 px, or its whole
  size if smaller) inside the chart area (`TargetOutOfChartArea`). Previously
  a start above the origin saturated to 0 and moved the opposite edge.
- The workspace is formatted with rustfmt, and `ci/verify.py static` (run by CI)
  now starts with `cargo fmt --all --check`.

## Patch — renderer 0.16.1 / model 0.10.0 / figgy 0.13.1 (2026-10-13)

- GPU resource retirement now transfers handles and their byte credits together
  through an atomic queue. Destruction and credit return still follow submission
  completion; retained prepared frames keep their resources alive.
- Bounded chart labels use bounded first-frame placement and size adjustments.
  Public model types, editing APIs and data-validation contracts are unchanged.
- Renderer tests independently verify selection borders and reset at 1×/2×,
  and directly pick all eight plot-area resize handles before applying each edit.
- Test-only stream adapters and accounting helpers no longer enter normal
  library builds. Remove an unused internal browser-prewarm wrapper; CI now
  rejects compiler and rustdoc warnings on all three platforms.
- API guidance distinguishes construction from retained pipeline prewarming,
  defines tracked-memory budget scope, and includes the required Cartesian
  axis/style refresh steps. The WASM example documents host-managed async access.
- The model remains 0.10.0. The WASM integration example advances to 0.13.1
  with the renderer dependency.

## First crates.io release — 2026-10-13

The first published registry versions are **figgy-renderer 0.16.1** and
**figgy-model 0.10.0**: the patch above and the packaging changes below. Rust
imports remain `renderer` and `model`; the renderer re-exports the model types.
The packaging changes preserve public API signatures and the model schema.

- The API landing pages start with installation and complete examples. The native
  PNG quick start includes the shipped `line` source directly and is compiled as
  a doctest. The model example creates and edits a chart without a GPU.
- docs.rs builds use `serde`, without GUI demo features. The renderer documents
  native Linux and WASM APIs; documentation compilation needs no graphics adapter.
- Registry archives retain code, shaders, fonts/notices, guides, examples and test
  fixtures. Gallery captures and unsupported host integrations stay in Git; the
  renderer archive is approximately 2 MiB instead of 9.2 MiB.
- The public Linux/Vulkan, Windows/DX12 and experimental macOS/Metal package matrix
  runs on non-draft PRs. Branch, merge and tag pushes do not run a duplicate matrix.
  Strict documentation checks cover both native and WASM APIs. Bounded chart
  regressions also exercise each supported decoration's public select/drag/reset
  path at 1×/2×, cache reuse, old-frame immutability and serialized state restoration.

Use the crates.io dependency in the [renderer quick start](crates/renderer/README.md#install-and-run)
or [model guide](crates/model/README.md#install). The registry dependencies
are `renderer = { package = "figgy-renderer", version = "0.16.1" }` and, for
model-only consumers, `model = { package = "figgy-model", version = "0.10.0" }`.
Native integration requires Rust 1.99 and wgpu 30. macOS/Metal remains experimental;
physical GPU driver certification is outside the software/virtual CI results.
The browser wrapper is an example with `publish = false`, not a registry product.

The published 0.16.1 renderer and 0.10.0 model archives still contain the earlier
unpublished/Git installation guidance. Their versioned docs.rs pages and crates.io
READMEs are generated from those archives, so source documentation corrections
and docs.rs rebuilds do not change them, and the old version URLs keep the
original text. Renderer 0.17.0 and model 0.11.0 (2026-10-16) carry the corrected
READMEs and API documentation.

## Source release — renderer 0.16.0 / model 0.10.0 / figgy 0.13.0

Radial, categorical and boxplot frames now expose the plot area through the same
`Selectable`, `Draggable` and eight-handle `Resizable` contracts as Cartesian
charts. Area edits rebuild geometry, axes, labels, picking and exports from the
model; they do not stretch a rendered image. `DecorationLayout.data_area` stores
an optional logical-pixel rectangle; `None` keeps automatic layout. See the
[workflow, hit priority and migration](crates/renderer/INTERACTION.md) and
[verification scope](ci/STATUS.md#renderer-0160-bounded-area-editing-2026-10-11).

### Previous patch — renderer 0.15.1 / model 0.9.0 / figgy 0.12.1

Radial outlines now include visible top/wall creases on raised pies and donuts.
Wood uses fine, irregular matte plywood grain on every face, without annual-ring
bands or repeated wall stripes. PNG regressions verify hover progress and persistent
selection at 1× and 2× on native and WebGPU. Public APIs and the model schema are
unchanged. See the [radial guide](crates/renderer/RADIAL.md) and
[verification scope](ci/STATUS.md#renderer-0151-radial-surfaces-and-export-2026-10-11).

### Previous feature release — renderer 0.15.0 / model 0.9.0 / figgy 0.12.0

Bounded chart titles, axes, tick/category labels, legends and data labels now expose
renderer-measured hit maps and position editing through the existing `Selectable` /
`Draggable` contracts. [Interaction workflow](crates/renderer/INTERACTION.md).
Rust struct literals that specify every bounded chart field must add
`decorations: Default::default()`; existing JSON defaults it automatically.
The `FiggyChart` JavaScript facade still does not expose bounded charts.
The native editors demonstrate selection and dragging; the radial editor starts
with the explicit 3D preset. Existing flat chart defaults remain unchanged.
See [verification scope](ci/STATUS.md#renderer-0150-bounded-chart-interaction-2026-10-11)
and this revision's Actions run for the three-OS result.

### Previous patch — figgy-renderer 0.14.1 / figgy-model 0.8.0 / figgy 0.11.3

Windows DX12 now uses explicit per-invocation initialization for the shared
arrays in extent reduction, picking and arc scans. This avoids WARP's first-use
compiler stall while preserving zero values and synchronization. Other backends,
public APIs and chart output contracts are unchanged. See the
[workaround and regression checks](ci/shader-compilation.md#windows-dx12-workgroup-initialization).

### Previous feature release — renderer 0.14.0 / figgy 0.11.2

Optional Cartesian pipelines can now compile in owned preparation jobs while the
renderer remains available for chart edits and frames. A short checked install
merges only GPU caches. Existing constructors and full-prewarm behavior remain
available. See the [preparation API](crates/renderer/API.md#detached-cartesian-pipeline-preparation).
The model and Config schema are unchanged.

### Previous patch — renderer 0.13.1 / figgy 0.11.1

This patch reduces shader compilation stalls and avoids mismatched browser prewarm
layouts. Contour labels retain the same four projection corrections, precision
handling and anti-aliasing. All production WGSL entries are covered by compilation
timing tests; repeated prewarm must create no new shaders or pipelines. Public APIs,
initialization progress events and the Config JSON schema are unchanged. See the
[measurements and validation scope](ci/shader-compilation.md).

### Feature baseline — renderer 0.13.0 / model 0.8.0 / figgy 0.11.0

This release adds bounded [pie/donut](crates/renderer/RADIAL.md),
[categorical bar](crates/renderer/CATEGORICAL.md) and [boxplot](crates/renderer/BOXPLOT.md)
renderers with editable models, procedural materials, picking and PNG output. Their
small-data contracts are separate from the Cartesian streaming API. Horizontal
histogram coverage, bounded stream-source caching and progressive replacement of the final
initial-fit preview are covered by renderer regression tests.

**Cargo package names change** to `figgy-renderer` and `figgy-model`; Rust imports remain
`renderer` and `model`. Add the corresponding `package` key to existing Git dependencies
when updating their revision. Use `features = ["serde"]` on the renderer to serialize
its re-exported model types. Rust 1.99 and wgpu 30 are the supported baseline.
These are source releases; neither crate has been published to crates.io. The WASM
wrapper remains `figgy` and has `publish = false` for the Rust registry. The new bounded
renderers have Rust APIs and are not exposed by the existing `FiggyChart` JS wrapper.

[CI and verification](ci/README.md) runs native builds, renderer/pixel regressions and
isolated package tests on standard Linux/Vulkan, Windows/DX12 and macOS/Metal runners.
The [recorded results and backend fixes](ci/STATUS.md) distinguish completed
checks from configured coverage.
**macOS / Metal support is experimental and not yet stable.** Renderer 0.14.0 fixes
the identified coordinate-rounding and midpoint-picking issues in shared WGSL.
Physical Apple GPU coverage remains outstanding; consult the exact revision's CI.
An opt-in [precompiled Metal shader experiment](ci/metal-precision-probe/README.md)
compares compiler behavior; the default renderer requires no precompiled shader or wgpu patch.
See [API contracts and migration](crates/renderer/API.md) for ownership and compatibility.

### Previous patch — renderer 0.12.6 / figgy 0.10.6

Streamed heatmaps now redraw when their colormap, z range, z scale or missing-value
color changes. Editing the top/right axes no longer replays unchanged data, while
an explicit auto-fit still restores all axes. Native regression tests edit an
already-rendered chart one SSOT property at a time, compare its first updated
frame with a fresh render, and check source requests, job identity and GPU
resource reuse. Public API signatures and the Config JSON schema are unchanged.

### Previous patch — renderer 0.12.5 / figgy 0.10.5

Resizing a window or changing its aspect ratio now reuses the view's packed GPU
rows when they cover the new geometry, including stroke and marker footprints.
The renderer requests original rows only when coverage is insufficient. Precise
streamed charts also apply grid visibility, color, width and line-style edits
without replaying data. Background-dependent styles rebuild when their grid
changes. Public API signatures and the Config JSON schema are unchanged.

### Previous patch — renderer 0.12.4 / figgy 0.10.4

Returning from a streaming chart to a resident chart now applies the current
window's display coordinates and clipping on the first frame. Queued stream
cancellation is processed before choosing the draw path, and cancellation alone
requests a redraw. Saved axis ranges are preserved. Public API signatures and
the Config JSON schema are unchanged; model remains 0.7.2.

### Previous patch — renderer 0.12.3 / figgy 0.10.3

Precise solid lines retain their visible connections to distant offscreen points,
including the first frame of a chart created with explicit axis ranges. Replacing
chart configuration, series or view, or cancelling a stream, clears its pending
fit request so a restored chart cannot inherit an earlier fit. Public API
signatures and the Config JSON schema are unchanged; model remains 0.7.2.
Update the Git revision and rebuild the browser package.

### Previous patch — renderer 0.12.2 / figgy 0.10.2

This patch stops repeated prefix replay during initial streamed fitting. Earlier
pixels are rescaled as new chunks arrive; one final exact replay replaces the
preview only when complete. Streaming upload buffers are reused and retired GPU
resources are explicitly destroyed after GPU completion. Numeric axis and colorbar
labels adapt to the available space without shrinking the font. Fitted tick settings
are committed with the final range, and progress no longer rewinds during fitting.
Wait for `complete` / `job.done`, not just the submitted count reaching the total.

Public API signatures and the Config JSON schema are unchanged; model remains
0.7.2. Update the Git revision and rebuild the browser package.

### Previous patch — renderer 0.12.1 / figgy 0.10.1

This patch fixes initial streamed auto-fit: partial data and axes use the same
progressively fitted range, internal fitting keeps its current job, and final
range commitment no longer starts a redundant execution. Ordinary redraws after
fitting preserve the user's range. Public API signatures and the Config JSON
schema are unchanged; model remains 0.7.2. See the
[initial-fit guide](crates/renderer/WASM.md#streaming-fit) for call order.
Update your Git revision and rebuild the browser package to use this fix.

### Previous feature release — renderer 0.12.0 / figgy 0.10.0

The 0.12.0 / 0.10.0 release added renderer-owned exact nonresident streaming and the browser
`render_chart()` job API. The web facade requests bounded original ranges,
schedules work, and reports progress. For supported precise point, solid-line,
and errorbar charts, the renderer can retain only the original rows needed by
the current view in a chart-local GPU cache; it never promotes the connected
whole-column closure automatically. Wider views and scaled export replay the
source, while a narrower view can redraw from that cache.
There is no LOD or downsampling. Supported styles and exclusions are listed in
[WASM.md](crates/renderer/WASM.md). Build the browser package from this
revision; the crates are distributed through this repository, not crates.io.
The model is version **0.7.2**, including the fix for subnormal logarithmic
contour levels. Renderer 0.12.0 and figgy 0.10.0 use wgpu 30 and Rust 1.99.0.

The previous renderer 0.11.0 / figgy 0.9.1 release introduced:

Subpixel histogram bins now fill each pixel column from zero to its maximum
bin value on the GPU. An enabled, nontransparent stroke supplies the entire
fill colour; otherwise the bin's fill colour is used. Original columns and
web API signatures are unchanged. The Rust `ColumnBarLayer` has a new
`envelope` field, so downstream struct literals require an update (`None`
for manually built layers without an envelope).
The browser demo includes bin-count and stroke controls. Build the package as
in [Build and run](crates/web/README.md#build-and-run), then run
`npx serve crates/web -l 8142` and open `http://localhost:8142/`.

Renderer 0.10.0 / figgy 0.9.0 introduced:

- **Histogram and matrix fields are first-class GPU series.** `Histogram` uses explicit bin edges/counts; `Heatmap`, `Contour`, and `HeatmapContour` share the declared matrix lattice and colour-map SSoT. Heatmaps support flat or interpolated shading, contours support up to 1024 levels, and contour labels open a real gap in the underlying isoline. Automatic field fitting uses the rendered cell boundaries rather than only the sample centres.
- **Field interaction and styling use stable identities.** `pick_data` returns tagged point, histogram-bin, matrix-cell, or contour-level references that can be written back through `Config.picked_data`. Histograms expose width, outline colour/thickness, and per-bin overrides. Contour label text/background/number formatting is independent of per-level line colour. The colourbar exposes its full `AxisOptions`, including ticks, labels, title, reversal, and pointer-following resize handles.
- **GPU range and startup contracts are exact and observable.** Hi/lo field-coordinate arithmetic and range reduction stay on the GPU; the reduced bounds committed to the axis SSoT are the same values used for drawing. Browser startup validates every render entry, including the contour-label width attribute, and `prewarm_all_with_progress` / `prewarm_all` can publish the renderer-owned lazy caches explicitly.
- **GPU memory accounting spans renderer resources.** Pool storage, staging, export, picking, and contour placement participate in allocation checks. Baked Milkyway/Constellation style textures remain an accounting exception; the reported total is not all GPU memory or driver VRAM usage. See [memory accounting](crates/renderer/ARCHITECTURE.md#memory-accounting-and-release).

This repository is the supported source distribution; the crates are not published on crates.io. Consumers pinned to a public Git revision must update their lockfile and rebuild the wasm package. See [WASM.md](crates/renderer/WASM.md) for browser lifecycle/API details and [SCHEMA.md](crates/web/SCHEMA.md) for the complete JSON contract.

---

# 변경 이력

아래 `figgy` 버전은 `crates/web`의 브라우저 통합 예제를 가리킨다.
Rust 렌더러의 버전 및 Studio 배포와는 구분한다.

아래 과거 소스 릴리스의 미배포 설명은 당시 상태다. 최초 crates.io 배포는
2026-10-13의 renderer 0.16.1 / model 0.10.0이며, 현재 설치법은 각 패키지의
README를 따른다. 이미 배포한 버전의 docs.rs·crates.io README는 소스 문서 수정이나
docs.rs 재빌드로 바뀌지 않으며, 기존 버전 URL은 원문을 유지한다. 수정된 설치 안내와
API 문서는 2026-10-16에 배포한 renderer 0.17.0 / model 0.11.0부터 반영됐다.

## 소스 릴리스 — renderer 0.16.0 / model 0.10.0 / figgy 0.13.0

원형·범주형 막대·박스플롯의 그리기 영역도 기존 `Selectable`, `Draggable`, `Resizable`로
선택하고 옮기거나 8개 핸들로 크기를 조정할 수 있다. 바뀐 영역에서 도형·축·라벨·피킹·PNG를
다시 계산한다. 이미 그린 이미지를 늘이지 않는다. `DecorationLayout.data_area`에 논리 픽셀
사각형을 저장하며, `None`이면 자동으로 배치한다.
[호출 순서와 호환성](crates/renderer/INTERACTION.md),
[검증 범위](ci/STATUS.md#renderer-0160-bounded-area-editing-2026-10-11)를 참고한다.

### 이전 패치 — renderer 0.15.1 / model 0.9.0 / figgy 0.12.1

입체 파이·도넛의 윗면과 옆면 사이에 빠졌던 경계선을 그린다. 우드 재질은 나이테와 반복되는
세로줄을 없애고, 모든 면에 미세하고 불규칙한 무광 합판 질감을 적용한다. 네이티브와 WebGPU의
1배·2배 PNG에서 호버 진행도와 클릭 선택에 따른 들림을 검사한다. 공개 API와 모델 형식은
그대로다. [원형 차트 사용법](crates/renderer/RADIAL.md)과
[검증 범위](ci/STATUS.md#renderer-0151-radial-surfaces-and-export-2026-10-11)를 참고한다.

### 이전 기능 릴리스 — renderer 0.15.0 / model 0.9.0 / figgy 0.12.0

원형·범주형 막대·박스플롯의 제목·축·눈금·범주 라벨·범례·값 라벨을 기존 선택·드래그 계약으로
편집할 수 있다. [사용 흐름과 호환성](crates/renderer/INTERACTION.md).
새 차트의 모든 필드를 명시하는 Rust 구조체 리터럴에는 `decorations: Default::default()`를
추가한다. 이전 JSON은 기본값으로 읽는다. 기존 `FiggyChart` JavaScript 클래스는
원형·범주형 막대·박스플롯 차트를 아직 지원하지 않는다. 네이티브 편집기에서
선택·드래그를 확인할 수 있으며, 원형 편집기는 명시적인 3D 프리셋으로 시작한다.
기존 차트의 평면 기본값은 유지한다.
[검증 범위](ci/STATUS.md#renderer-0150-bounded-chart-interaction-2026-10-11)와
해당 리비전의 Actions 실행에서 세 OS 결과를 확인할 수 있다.

### 이전 패치 — figgy-renderer 0.14.1 / figgy-model 0.8.0 / figgy 0.11.3

Windows DX12에서는 범위 계산·피킹·호장 스캔의 공유 배열을 각 실행 단위가 나눠서
초기화한다. 초기값과 동기화 보장을 유지하면서 WARP의 첫 실행 컴파일 지연을 피한다.
다른 백엔드와 공개 API, 차트 출력 계약은 그대로다.
[우회 방식과 회귀 검사](ci/shader-compilation.md#windows-dx12-workgroup-initialization)를 참고한다.

### 이전 기능 릴리스 — renderer 0.14.0 / figgy 0.11.2

직교 차트의 기능별 파이프라인을 렌더러와 분리된 준비 작업에서 컴파일하는 동안에도 차트를
계속 편집하고 그릴 수 있다.
준비 결과를 설치할 때는 같은 렌더러와 출력 대상인지 확인한 뒤 GPU 캐시만 합친다.
기존 생성자와 전체 예열 API도 그대로 사용할 수 있다.
사용 계약은 [파이프라인 준비 API](crates/renderer/API.md#detached-cartesian-pipeline-preparation)를 참고한다.
모델과 Config 스키마는 바뀌지 않았다.

### 이전 패치 — renderer 0.13.1 / figgy 0.11.1

이번 패치는 셰이더 컴파일 지연과 브라우저의 중복 컴파일을 줄인다. 등고선 라벨의
4회 위치 보정, 정밀 계산, 안티앨리어싱은 유지한다. 모든 셰이더 진입점의 컴파일 시간을
검사하고, 예열을 다시 호출할 때 셰이더나 파이프라인을 새로 만들지 않는지도 확인한다.
공개 API, 초기화 진행 이벤트, Config JSON 형식은 바뀌지 않았다.
[측정 결과와 검증 범위](ci/shader-compilation.md)를 참고한다.

### 기능 기준 버전 — renderer 0.13.0 / model 0.8.0 / figgy 0.11.0

[파이·도넛](crates/renderer/RADIAL.md), [범주형 막대](crates/renderer/CATEGORICAL.md),
[박스플롯](crates/renderer/BOXPLOT.md) 렌더러를 추가했다. 개별 항목 편집, 셰이더 질감,
선택과 PNG 출력을 지원하며, 대용량 스트리밍과는 별도의 소량 데이터 계약을 사용한다.
가로 히스토그램, 스트림 원본 구간의 제한된 캐시, 초기 범위 맞춤 마지막 단계의 순차 화면 갱신도
렌더러 회귀 테스트로 검사한다.

**Cargo 패키지명은 `figgy-renderer`·`figgy-model`로 바뀐다.** Rust 코드의 `renderer`·`model`
이름은 유지한다. 기존 Git 의존성을 갱신할 때는 선택한 리비전과 함께 `package` 키를 추가한다.
직렬화가 필요하면 렌더러에 `features = ["serde"]`를 지정하면 된다. 지원 기준은 Rust 1.99와
wgpu 30이다. 아직 crates.io에 게시하지 않았으며, WASM 래퍼 `figgy`는 Rust registry 게시
대상에서 제외했다. 새 소량 데이터 렌더러는 Rust API로 제공하며, 기존 `FiggyChart` JS 래퍼에는
노출하지 않는다.

세 OS의 빌드·렌더링·픽셀·독립 패키지 검사와 재실행 방법은 [CI 안내](ci/README.md)를 참고한다.
[실행 결과와 백엔드 수정 내역](ci/STATUS.md)에서 검증 범위를 확인할 수 있다.
**macOS / Metal 지원은 아직 불안정한 실험 단계다.** 0.14.0에서는 좌표 계산의 반올림과
선분 중점 피킹 문제를 공통 WGSL에서 수정했다. 실제 Apple GPU 검증은 아직 남아 있으므로
사용할 리비전의 CI 결과를 확인해야 한다. [정밀 Metal 셰이더 실험](ci/metal-precision-probe/README.md)은
컴파일러 동작을 비교하는 별도 검사다. 기본 렌더러에는 미리 컴파일한 셰이더나 wgpu 패치가 필요 없다.
[API 계약](crates/renderer/API.md)에서 소유권과 마이그레이션 방법을 확인할 수 있다.

### 이전 패치 — renderer 0.12.6 / figgy 0.10.6

스트리밍 히트맵의 색상표·Z 범위·Z 스케일·결측값 색을 바꾸면 데이터 화면도 갱신한다. 위쪽·오른쪽 축만 바꿀 때는 데이터를 다시 읽지 않으며, 자동 맞춤을 명시적으로 호출하면 이전처럼 모든 축을 복원한다. 네이티브 회귀 테스트는 이미 그린 차트의 SSOT 설정을 하나씩 바꿔 첫 갱신 화면을 새 차트와 비교하고, 원본 요청·작업 ID·GPU 자원 재사용도 검사한다. 공개 API 형식과 Config JSON 스키마는 그대로다.

### 이전 패치 — renderer 0.12.5 / figgy 0.10.5

창 크기나 가로세로 비율을 바꿔도, 선 두께·점 크기까지 포함한 새 화면의 필요 영역이 GPU 캐시에 있으면 원본을 다시 읽지 않는다. 캐시 범위가 부족할 때만 원본 구간을 다시 요청한다. 정밀 스타일의 스트림 차트에서는 그리드 표시·색·두께·선 모양 변경도 데이터 재생 없이 반영한다. 배경과 합성하는 방식이 다른 스타일은 그리드 변경 시 다시 그린다. 공개 API 형식과 Config JSON 스키마는 바뀌지 않았다.

### 이전 패치 — renderer 0.12.4 / figgy 0.10.4

스트림 차트에서 상주 차트로 돌아올 때 첫 화면의 선이 축 밖으로 새거나 중간에서 잘리던 문제를 고쳤다. 그리기 경로를 선택하기 전에 예약된 스트림 취소를 처리해, 복귀 즉시 창 크기에 맞는 좌표와 잘라내기 영역을 적용한다. 취소만 해도 다음 프레임을 예약하며, 저장된 축 범위는 바꾸지 않는다. 공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다.

### 이전 패치 — renderer 0.12.3 / figgy 0.10.3

정밀 스타일의 실선에서 화면 밖의 먼 점으로 이어지는 선이 사라지는 문제를 고쳤다. 자동 맞춤 없이 범위를 지정한 새 차트의 첫 화면에서도 연결선을 표시한다. 설정·시리즈·표시 범위를 교체하거나 스트림을 취소하면 이전 자동 맞춤 요청을 해제해, 복원한 차트에 이전 요청이 남지 않도록 했다.

공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다. Git 리비전을 갱신하고 브라우저 패키지를 다시 빌드해야 적용된다.

### 이전 패치 — renderer 0.12.2 / figgy 0.10.2

초기 스트림 맞춤 중 앞부분을 반복해서 읽고 그리던 문제를 고쳤다. 먼저 그린 데이터는 새 범위로 리스케일하고 새 청크를 이어 그린다. 마지막에는 원본으로 한 번 정확히 다시 그려, 완성된 화면으로 교체한다. 업로드 버퍼는 재사용하며 사용이 끝난 GPU 자원은 GPU 작업 완료 후 명시적으로 해제한다.

숫자 눈금과 색상 막대 라벨은 글씨 크기를 유지하면서 화면 공간에 맞춰 표시 간격을 조절한다. 맞춤이 끝나면 축 범위와 눈금 설정을 함께 반영한다. 진행량은 되감기지 않으며, 제출량이 전체량에 도달했더라도 `complete` 또는 `job.done`까지 기다려야 한다.

공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다. 사용하려면 Git 리비전을 갱신하고 브라우저 패키지를 다시 빌드해야 한다.

### 이전 패치 — renderer 0.12.1 / figgy 0.10.1

첫 스트림의 자동 맞춤을 수정한 패치 버전이다. 중간 결과의 데이터와 축이 같은 범위를 사용하며, 내부 맞춤 과정에서 작업이 반복 교체되거나 최종 범위 반영 뒤 불필요하게 다시 시작되는 문제를 고쳤다. 맞춤이 끝난 뒤 일반 재그리기는 사용자가 정한 범위를 유지한다.

공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다. 호출 순서는 [범위 맞춤 사용법](crates/renderer/WASM.md#streaming-fit)을 참고한다. 수정 사항을 적용하려면 Git 리비전을 갱신하고 브라우저 패키지를 다시 빌드해야 한다.

### 이전 기능 릴리스 — renderer 0.12.0 / figgy 0.10.0

0.12.0 / 0.10.0에는 원본 데이터를 나누어 그리는 스트리밍 기능과 웹 작업 API인 `render_chart()`가 추가됐다. 렌더러가 필요한 데이터 구간을 요청하면 웹 래퍼가 원본을 공급하고 작업 순서와 진행 상태를 관리한다.

지원되는 정밀 모드의 점·실선·오차 막대 차트에서는 현재 화면에 필요한 원본 행만 차트별 GPU 캐시에 보관할 수 있다. 서로 연결된 컬럼 전체를 자동으로 상주 풀에 옮기지는 않는다. 완료된 결과는 재사용하며, 표시 범위를 넓히거나 출력 배율을 바꾸면 같은 원본을 다시 읽어 그린다. LOD나 다운샘플링으로 데이터를 줄이지 않는다.

지원 범위와 제한 사항은 [WASM.md](crates/renderer/WASM.md)를 참고한다. 브라우저 패키지는 사용할 소스 커밋에서 빌드하며, 크레이트는 crates.io에 배포하지 않는다.

모델은 로그 등고선의 매우 작은 값 처리 수정을 포함한 **0.7.2**다. 렌더러와 웹 패키지는 wgpu 30, 개발 툴체인은 Rust 1.99.0을 사용한다.

이전 renderer 0.11.0 / figgy 0.9.1 릴리스의 변경 내용:

화면에서 폭이 1픽셀보다 좁은 히스토그램 구간은 GPU가 픽셀 열별 최댓값을 골라 0까지 채운다. 외곽선 두께와 불투명도가 모두 양수이면 외곽선 색으로, 그렇지 않으면 채움색으로 영역 전체를 그린다.
원본 컬럼과 웹 API 형식은 그대로다. Rust의 `ColumnBarLayer`에는 `envelope` 필드가 추가됐으므로 구조체를 직접 생성하는 코드는 이 필드를 지정해야 한다. 수동으로 만든 레이어에 해당 자원이 없으면 `None`을 사용한다.
데모에는 구간 수 조절 슬라이더와 외곽선 표시 옵션이 있다. [빌드와 실행](crates/web/README.md#build-and-run) 절에 따라 브라우저 패키지를 빌드한 뒤 `npx serve crates/web -l 8142`를 실행하고 `http://localhost:8142/`에서 확인할 수 있다.

renderer 0.10.0 / figgy 0.9.0 릴리스에 포함된 기능은 다음과 같다.

- **히스토그램과 행렬 데이터를 GPU에서 그린다.** `Histogram`은 구간 경계와 빈도 값을 명시적으로 받는다. `Heatmap`, `Contour`, `HeatmapContour`는 같은 행렬 격자와 색상표 설정을 사용한다. 히트맵은 셀 단위 채움과 보간을 지원하고, 등고선은 최대 1024개 레벨을 지원한다. 등고선 라벨이 놓인 곳에서는 선을 끊어 글자가 읽히도록 한다. 자동 범위 맞춤은 실제로 그려지는 셀 경계를 기준으로 한다.
- **선택한 데이터는 식별자로 추적한다.** `pick_data`는 점·히스토그램 구간·행렬 셀·등고선 레벨을 구분하는 참조를 반환한다. 이 참조를 `Config.picked_data`에 넣으면 해당 데이터를 강조할 수 있다. 히스토그램은 막대 폭, 외곽선 색·두께, 구간별 스타일을 지원한다. 등고선 라벨의 글자색·배경·숫자 형식은 선 색과 별도로 설정한다. 색상 막대는 눈금·라벨·제목·방향 반전과 크기 조절을 포함한 전체 `AxisOptions`를 제공한다.
- **범위 계산과 초기화 과정을 확인할 수 있다.** 행렬 좌표의 hi/lo 연산과 범위 계산은 GPU에서 수행한다. 축 설정에 기록하는 범위는 실제 그리기에 사용하는 값과 같다. 브라우저 초기화에서는 등고선 라벨의 너비 속성을 포함해 모든 렌더링 진입점을 검증한다. `prewarm_all_with_progress` 또는 `prewarm_all`을 호출하면 필요할 때 생성되는 렌더러 캐시를 미리 준비할 수 있다.
- **주요 GPU 자원의 할당량을 추적한다.** 풀 저장소, 업로드용 스테이징 버퍼, 이미지 출력, 피킹, 등고선 라벨 배치에 메모리 예산 검사를 적용한다. 은하수·별자리 스타일의 미리 생성한 텍스처는 일반 집계에서 제외된다. 따라서 보고값은 전체 GPU 메모리나 실제 VRAM 사용량과 다르다. 자세한 범위는 [메모리 계상과 회수](crates/renderer/ARCHITECTURE.md#메모리-계상과-회수)를 참고한다.

이 라이브러리는 공개 저장소의 소스로 제공하며 crates.io에는 배포하지 않는다. 특정 공개 Git 커밋을 사용하는 프로젝트는 버전을 바꿀 때 잠금 파일을 갱신하고 WASM 패키지를 다시 빌드해야 한다. 브라우저 API와 수명 관리는 [WASM.md](crates/renderer/WASM.md), JSON 형식은 [SCHEMA.md](crates/web/SCHEMA.md)에 설명돼 있다.
