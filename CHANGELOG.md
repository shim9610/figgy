# Release history

`figgy-renderer` and `figgy-model` are the Rust library packages. `figgy` in this
history is the separate browser integration example in `crates/web`; its version
does not imply a standalone JavaScript product or a Studio deployment.

Publication statements in older source-release entries describe those releases
at the time. The first crates.io publication is recorded below.

[Usage and gallery](README.md) · [Backend verification](ci/STATUS.md)

## Unreleased — subplots

- Subplots (issue #11, work item 5): `Config` gains `subplots: Vec<Subplot>`,
  more charts drawn in the same document. A `Subplot { config, series,
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
- Interaction: `LayerRef` gains `Subplot(i)` and `SubplotOverlay(i, j)`, with
  the same out-of-range recovery as `Overlay`. While one is active
  `HitMap::for_active_layer` registers exactly that subplot's own set,
  wrapped as `SubplotElement`s (`subplot_<i>:…` ids); drags and resizes use
  the single-chart policies on the subplot's chart and land in
  `subplots[i].config`. The WASM wrapper accepts the new layers, rebuilds its
  hit map when the active subplot's overlay count changes and adds
  `auto_fit_subplot(index, "x" | "y", columns, padding)`. New native example
  `subplots` (a 2×2 grid with one linked X axis and one twin Y axis).
- Rust migration: exhaustive `Config` literals need `subplots` (or
  `..Default::default()`), `ChartViewState` literals need `subplots`, and
  exhaustive matches on `LayerRef` need the two new variants (`WarningAxis` is
  `non_exhaustive`). Released with the next renderer/model minor versions
  together with the other unreleased entries. See the configuration reference
  (`Subplot`), the interaction guide (subplots) and the WASM guide.

## Unreleased — overlay layers

- `Config` gains `overlays: Vec<OverlayLayer>` (issue #11, work items 1–4):
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
  (also on `WindowedRenderer` and the WASM wrapper as JSON) store the layer,
  clear the element selection on a change and recover an out-of-range index to
  `Base` through every registry entry point. `HitMap::for_active_layer(cfg)`
  registers the shared elements first and the active overlay's
  `OverlayAxisElement` / `OverlayAxisLabelElement` / `OverlayAxisTitleElement` /
  `OverlayLegendElement` last (`overlay_<n>:…` ids); their drags write back
  into `overlays[n]` only. GPU picking addresses the active layer's panel and
  series. The WASM wrapper rebuilds its hit map when the layer or overlay
  count changes and adds `auto_fit_overlay_y`.
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
  `export_panel_*` stay and render the base panel. The WASM `export_png`
  exports the document. New native example `overlay_axes`.
- Limits by design: overlays own no X axis, nest one level, exclude bounded
  charts, and a nonresident streaming series listed by an overlay is rejected
  (`InvalidSeriesConfig`); the stream request paths reject a document with
  overlays (`InvalidConfig { field: "overlays" }`).
- Rust migration: exhaustive `Config` literals need `overlays` (or
  `..Default::default()`), `ChartViewState` literals need `overlay_y`, and
  exhaustive matches on `WarningAxis` need `OverlayY`. Released with the next
  renderer/model minor versions together with the other unreleased entries.
  See the interaction guide, the configuration reference and the WASM guide.

## Unreleased — annotation layer

- Add the dedicated annotation layer to every chart family (#12, steps 1–4).
  `AnnotationLayer` gains `items: Vec<Annotation>`; an `Annotation` is an
  `id`, a `visible` flag and one of `ReferenceLine { axis, value, style, label }`,
  `Span { axis, from, to, fill }`, `Text { anchor, text, box_style }` or
  `Arrow { from, to, style, text }`, with `Anchor::Data` (axis coordinates that
  follow zoom and range changes) or `Anchor::Ratio` (plot-area fractions, fixed
  on screen). `AnnotationAxis::X` / `Y` are screen axes in every family:
  `bottom_x` / `left_y` on Cartesian charts, the category index axis and the
  value axis on bar and box charts, no value axes on radial charts. Non-finite
  values and unplaceable anchors are not errors: the raster skips the item and
  reports `RenderWarning::AnnotationSkipped { index, reason }`
  (`AnnotationSkip::{NonFinite, NoValueAxis, OutOfRange, EmptyText}`);
  `RenderWarning::axis()` now returns `Option<WarningAxis>`. Malformed styles
  are rejected by `AnnotationLayer::validate`, part of config validation.
  Annotations never change the data-area margins and are not nudged by the
  label placer. The JSON shape is additive: an empty `items` is omitted.
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
  the `AnnotationSpace` trait).
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
- Pointer route by the active layer (#12, step 4): while
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
  `set_active_layer` (an `Overlay` reads and is stored as `Base`; a change
  clears `decorations.selected` and keeps the data selection), and
  `LayerRef::bounded()`. A layer switch reuses the data, grid and decoration
  textures and the bounded frames; routed selection and drags refresh the
  annotation layer only.
- The WASM wrappers `FiggyChart` and `FiggyBoundedChart` expose
  `annotations`, `hit_annotation`, `select_annotation`, `selected_annotation`,
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
- Tests: the SSOT invalidation matrix classifies `annotations.items` (annotation
  revision only) and `annotations.visible` (draw only) on resident and streamed
  charts; the Cartesian layer tests prove that an annotation refresh rewrites
  neither the grid nor the decoration texture and that the cached view paints
  the bytes of a fresh renderer, that spans sit below the data and text boxes
  above the legend, that drags round-trip, and that 1× / 2× exports share
  metrics with the hit map; the bounded tests prove the same frame reuse,
  export parity, selection, drags and radial data-anchor warnings for all
  three families. The routing tests register per-layer sets for single-layer
  and layered Cartesian documents, put the items above the legend only while
  `Annotations` is active (1× and 2×), pass hidden layers through, keep every
  grid, decoration and bounded frame cache across a layer switch, paint the
  bytes of a fresh renderer after routed selection and handle drags, and keep
  anchor kinds through JSON for all three bounded families. Guides:
  configuration reference, interaction guide
  (annotation layer), architecture (revision table), bounded family guides,
  WASM guide and JSON schema.
- Rust migration: exhaustive `AnnotationLayer` literals need `items`;
  exhaustive matches on `RenderWarning` need `AnnotationSkipped`, and
  `RenderWarning::axis()` is an `Option`. `draw_chart_panel_columnar` takes
  the two optional annotation layers. Released with the next renderer/model
  minor versions together with the other Unreleased entries below.

## Unreleased — area fill and step lines

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
- Constraints, documented and tested rather than masked: fills assume
  x-monotonic series (a doubled-back series blends its overlapping
  trapezoids twice); fill between two series requires the same x column and
  rejects a different column, an unknown series or a self reference with
  `InvalidSeriesConfig`; both fills and step modes require GPU-resident
  columns and are rejected on streamed sources by
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
  The GPU picker's query uniform gains the step mode (240 bytes); the pick
  registry signature includes it, so a step edit rebuilds only that series'
  pick slot. `DataLineStyleConfig` implements `Default`.
- Rust migration: exhaustive `DataLineStyleConfig` literals need `fill`,
  `fill_color` and `interpolation` (or `..Default::default()`); exhaustive
  `RichSegment` literals need `rule_band`. This public field addition is
  released with the next renderer/model minor versions together with the
  other Unreleased entries below. See the configuration reference
  ("Area fill and step lines") and the interaction guide.

## Unreleased — layer and annotation foundation

- Add the SSOT shape shared by the planned overlay layers (secondary Y axes,
  subplots) and the dedicated annotation layer, without any behavior change.
  `Config`, `RadialChart`, `CategoricalChart` and `BoxPlotChart` gain
  `active_layer: LayerRef` (`Base` default, `Overlay(index)`, `Annotations`)
  and `annotations: AnnotationLayer { visible }`. Both are `serde(default)` and
  omitted from JSON at their defaults, so existing documents parse and
  serialize byte-identically. `DecorationId` gains `Overlay { layer, inner }`
  (`overlay_<n>:<inner id>`) and `Annotation(index)` (`annotation_<n>`); no hit
  map registers them yet. Drawn pixels do not depend on either field, and the
  bounded frame and label raster caches exclude both from their keys, so
  editing them reuses the cached frame. See the interaction guide and
  configuration reference.
- The registry's revisions now follow a single exhaustive classification of
  `Config` fields instead of refreshing the axis raster on every config
  replacement. `set_chart_config` and `set_chart_state` leave every revision
  unchanged for an identical config, bump `desired` and `config` for any
  change, bump `raster` only for chrome fields (`raster_config_equal`: chart
  area, axes, title, grid, legend, colourbar, draw style, plus any series
  change), `view` for data geometry (`stream_config_equal`) and `selection`
  for `picked_*`. The SSOT invalidation matrix asserts those revisions and the
  pixel oracle for every `Config` field, now including `chart_area.x` / `y`,
  `label_font`, `significant_digits`, the Milkyway and Constellation styles
  and every colourbar field, behind an exhaustive destructuring guard so a new
  field cannot land unclassified. Hosts that relied on an identical
  `set_chart_config` call to force a raster refresh should edit the chart or
  refresh the view directly.
- Rust migration: exhaustive chart struct literals need `active_layer` and
  `annotations` (or `..Default::default()`); exhaustive matches on
  `DecorationId` need the two new arms. This public field addition is released
  with the next renderer/model minor versions together with the bounded title
  change below.

## Unreleased — independent bounded titles

- Restore the bounded rich-title patches previously used by the Studio Git
  consumer (`83a5c83`, `40de171`) on the published 0.17.0 code line.
  `RadialChart`, `CategoricalChart` and `BoxPlotChart` gain optional
  `chart_title: Option<BoundedTitleOptions>` with independent visibility and
  `RichText` (font, size, color, segment styles and `TextAlign`). Legacy JSON
  without the field still uses the plain `title` and shared label style.
  Editors call `normalize_title()` once before editing labels to preserve
  the original title appearance. Title hit bounds, dragging and export use
  the same renderer metrics at every output scale.
- Rust migration: exhaustive chart struct literals need `chart_title: None`;
  literals using `..Default::default()` remain valid. This public field addition
  requires coordinated release versions; this PR does not publish or relabel
  renderer 0.17.0/model 0.11.0. See the interaction guide and `bounded_titles`
  example. The existing label collision handling is retained.

## Upcoming release — renderer 0.17.0 / model 0.11.0 / figgy 0.14.0

New 0.x minor versions: `RichText.align` is a new public field, so fully
specified `RichText` literals need the migration noted below. JSON without
`align` still parses. The WASM integration example
advances to 0.14.0 with the renderer dependency. This entry does not itself
publish a registry crate. As verified on 2026-10-16, crates.io still contains
renderer 0.16.1 and model 0.10.0; this entry describes the release being prepared.

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

## Source patch — renderer 0.16.1 / model 0.10.0 / figgy 0.13.1

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
  with the renderer dependency; this does not publish a registry crate or Studio.

## First crates.io release — 2026-10-13

The first published registry versions are **figgy-renderer 0.16.1** and
**figgy-model 0.10.0**. Rust imports remain `renderer` and `model`; the renderer
re-exports the model types. The patch below preserves public API signatures and
the model schema.

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
READMEs are generated from those archives; merging these source documentation
corrections or requesting a docs.rs rebuild cannot replace them. Corrected registry
READMEs and API documentation require a subsequent release of each affected crate.
A documentation-only patch release is sufficient; no public API, model schema or
rendering change is needed. The old version URLs will retain the original text.
This source correction does not publish or increment either package.

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
Online Studio deployment is separate from this source release.

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
Update the Git revision and rebuild the browser package. Studio deployment is
separate from this source release.

### Previous patch — renderer 0.12.2 / figgy 0.10.2

This patch stops repeated prefix replay during initial streamed fitting. Earlier
pixels are rescaled as new chunks arrive; one final exact replay replaces the
preview only when complete. Streaming upload buffers are reused and retired GPU
resources are explicitly destroyed after GPU completion. Numeric axis and colorbar
labels adapt to the available space without shrinking the font. Fitted tick settings
are committed with the final range, and progress no longer rewinds during fitting.
Wait for `complete` / `job.done`, not just the submitted count reaching the total.

Public API signatures and the Config JSON schema are unchanged; model remains
0.7.2. Update the Git revision and rebuild the browser package. Online Studio
deployment is separate from this source release.

### Previous patch — renderer 0.12.1 / figgy 0.10.1

This patch fixes initial streamed auto-fit: partial data and axes use the same
progressively fitted range, internal fitting keeps its current job, and final
range commitment no longer starts a redundant execution. Ordinary redraws after
fitting preserve the user's range. Public API signatures and the Config JSON
schema are unchanged; model remains 0.7.2. See the
[initial-fit guide](crates/renderer/WASM.md#streaming-fit) for call order.
Update your Git revision and rebuild the browser package to use this fix.
Publishing this source does not deploy the online Studio.

### Previous feature release — renderer 0.12.0 / figgy 0.10.0

The 0.12.0 / 0.10.0 release added renderer-owned exact nonresident streaming and the browser
`render_chart()` job API. The web facade requests bounded original ranges,
schedules work, and reports progress. For supported precise point, solid-line,
and errorbar charts, the renderer can retain only the original rows needed by
the current view in a chart-local GPU cache; it never promotes the connected
whole-column closure automatically. Wider views and scaled export replay the
source, while a narrower view can redraw from that cache.
There is no LOD or downsampling. Supported styles and exclusions are listed in
[WASM.md](crates/renderer/WASM.md). This source release does **not** mean the
online Studio has adopted the new API. Build the browser package from this
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
The browser demo includes bin-count and stroke controls. Run
`npx serve crates/web -l 8142`, then open `http://localhost:8142/`.

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
docs.rs 재빌드로 바뀌지 않는다. 수정된 공개 문서는 각 패키지의 후속 배포에 반영되며,
문서만 고치는 패치 릴리스로 충분하다. 기존 버전 URL은 원문을 유지한다.

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
추가한다. 이전 JSON은 기본값으로 읽는다. 기존 `FiggyChart` JavaScript 클래스의 새 차트
연결은 별도 작업이다. 네이티브 편집기에서 선택·드래그를 확인할 수 있으며, 원형 편집기는
명시적인 3D 프리셋으로 시작한다. 기존 차트의 평면 기본값은 유지한다.
[검증 범위](ci/STATUS.md#renderer-0150-bounded-chart-interaction-2026-10-11)와
해당 리비전의 Actions 실행에서 세 OS 결과를 확인할 수 있다.

### 이전 패치 — figgy-renderer 0.14.1 / figgy-model 0.8.0 / figgy 0.11.3

Windows DX12에서는 범위 계산·피킹·호장 스캔의 공유 배열을 각 실행 단위가 나눠서
초기화한다. 초기값과 동기화 보장을 유지하면서 WARP의 첫 실행 컴파일 지연을 피한다.
다른 백엔드와 공개 API, 차트 출력 계약은 그대로다.
[우회 방식과 회귀 검사](ci/shader-compilation.md#windows-dx12-workgroup-initialization)에 근거를 정리했다.

### 이전 기능 릴리스 — renderer 0.14.0 / figgy 0.11.2

선택 기능의 파이프라인을 별도 작업으로 준비하면서 차트를 계속 편집하고 그릴 수 있다.
준비 결과를 설치할 때는 같은 렌더러와 출력 대상인지 확인한 뒤 GPU 캐시만 합친다.
기존 생성자와 전체 예열 API도 그대로 사용할 수 있다.
[파이프라인 준비 API](crates/renderer/API.md#detached-cartesian-pipeline-preparation)에 사용 계약을 정리했다.
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
가로 히스토그램, 스트림 원본 구간의 제한된 캐시, 초기 fit 마지막 단계의 순차 화면 갱신도
렌더러 회귀 테스트로 검사한다.

**Cargo 패키지명은 `figgy-renderer`·`figgy-model`로 바뀐다.** Rust 코드의 `renderer`·`model`
이름은 유지한다. 기존 Git 의존성을 갱신할 때는 선택한 리비전과 함께 `package` 키를 추가한다.
직렬화가 필요하면 렌더러에 `features = ["serde"]`를 지정하면 된다. 지원 기준은 Rust 1.99와
wgpu 30이다. 아직 crates.io에 게시하지 않았으며, WASM 래퍼 `figgy`는 Rust registry 게시
대상에서 제외했다. 새 소량 데이터 렌더러는 Rust API로 제공하며, 기존 `FiggyChart` JS 래퍼에는
노출하지 않는다.

[CI 안내](ci/README.md)에 세 OS의 빌드·렌더링·픽셀·독립 패키지 검사와 재실행 방법을 정리했다.
[실행 결과와 백엔드 수정 내역](ci/STATUS.md)에서 검증 범위를 확인할 수 있다.
**macOS / Metal 지원은 아직 불안정한 실험 단계다.** 0.14.0에서는 좌표 계산의 반올림과
선분 중점 피킹 문제를 공통 WGSL에서 수정했다. 실제 Apple GPU 검증은 아직 남아 있으므로
사용할 리비전의 CI 결과를 확인해야 한다. [정밀 Metal 셰이더 실험](ci/metal-precision-probe/README.md)은
컴파일러 동작을 비교하는 별도 검사다. 기본 렌더러에는 미리 컴파일한 셰이더나 wgpu 패치가 필요 없다.
[API 계약](crates/renderer/API.md)에서 소유권과 마이그레이션 방법을 확인할 수 있다.
온라인 스튜디오 배포는 이번 소스 공개와 별개다.

### 이전 패치 — renderer 0.12.6 / figgy 0.10.6

스트리밍 히트맵의 색상표·Z 범위·Z 스케일·결측값 색을 바꾸면 데이터 화면도 갱신한다. 위쪽·오른쪽 축만 바꿀 때는 데이터를 다시 읽지 않으며, 명시적으로 오토핏을 호출하면 모든 축을 정상 복원한다. 네이티브 회귀 테스트는 이미 그린 차트의 SSOT 설정을 하나씩 바꿔 첫 갱신 화면을 새 차트와 비교하고, 원본 요청·작업 ID·GPU 자원 재사용도 검사한다. 공개 API 형식과 Config JSON 스키마는 그대로다.

### 이전 패치 — renderer 0.12.5 / figgy 0.10.5

창 크기나 가로세로 비율을 바꿔도, 선 두께·점 크기까지 포함한 새 화면의 필요 영역이 GPU 캐시에 있으면 원본을 다시 읽지 않는다. 캐시 범위가 부족할 때만 원본 구간을 다시 요청한다. 정밀 스타일의 스트림 차트에서는 그리드 표시·색·두께·선 모양 변경도 데이터 재생 없이 반영한다. 배경과 합성하는 방식이 다른 스타일은 그리드 변경 시 다시 그린다. 공개 API 형식과 Config JSON 스키마는 바뀌지 않았다.

### 이전 패치 — renderer 0.12.4 / figgy 0.10.4

스트림 차트에서 상주 차트로 돌아올 때 첫 화면의 선이 축 밖으로 새거나 중간에서 잘리던 문제를 고쳤다. 그리기 경로를 선택하기 전에 예약된 스트림 취소를 처리해, 복귀 즉시 창 크기에 맞는 좌표와 잘라내기 영역을 적용한다. 취소만 해도 다음 프레임을 예약하며, 저장된 축 범위는 바꾸지 않는다. 공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다.

### 이전 패치 — renderer 0.12.3 / figgy 0.10.3

정밀 스타일의 실선에서 화면 밖의 먼 점으로 이어지는 선이 사라지는 문제를 고쳤다. 자동 맞춤 없이 범위를 지정한 새 차트의 첫 화면에서도 연결선을 표시한다. 설정·시리즈·표시 범위를 교체하거나 스트림을 취소하면 이전 자동 맞춤 요청을 해제해, 복원한 차트에 이전 요청이 남지 않도록 했다.

공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다. Git 리비전을 갱신하고 브라우저 패키지를 다시 빌드해야 적용된다. 온라인 스튜디오 배포는 소스 릴리스와 별개다.

### 이전 패치 — renderer 0.12.2 / figgy 0.10.2

초기 스트림 맞춤 중 앞부분을 반복해서 읽고 그리던 문제를 고쳤다. 먼저 그린 데이터는 새 범위로 리스케일하고 새 청크를 이어 그린다. 마지막에는 원본으로 한 번 정확히 다시 그려, 완성된 화면으로 교체한다. 업로드 버퍼는 재사용하며 사용이 끝난 GPU 자원은 GPU 작업 완료 후 명시적으로 해제한다.

숫자 눈금과 색상 막대 라벨은 글씨 크기를 유지하면서 화면 공간에 맞춰 표시 간격을 조절한다. 맞춤이 끝나면 축 범위와 눈금 설정을 함께 반영한다. 진행량은 되감기지 않으며, 제출량이 전체량에 도달했더라도 `complete` 또는 `job.done`까지 기다려야 한다.

공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다. 사용하려면 Git 리비전을 갱신하고 브라우저 패키지를 다시 빌드해야 한다. 온라인 스튜디오 배포는 소스 릴리스와 별개다.

### 이전 패치 — renderer 0.12.1 / figgy 0.10.1

첫 스트림의 자동 맞춤을 수정한 패치 버전이다. 중간 결과의 데이터와 축이 같은 범위를 사용하며, 내부 맞춤 과정에서 작업이 반복 교체되거나 최종 범위 반영 뒤 불필요하게 다시 시작되는 문제를 고쳤다. 맞춤이 끝난 뒤 일반 재그리기는 사용자가 정한 범위를 유지한다.

공개 API 형식과 Config JSON 스키마는 그대로이며 model은 0.7.2를 유지한다. 호출 순서는 [범위 맞춤 사용법](crates/renderer/WASM.md#streaming-fit)을 참고한다. 수정 사항을 적용하려면 Git 리비전을 갱신하고 브라우저 패키지를 다시 빌드해야 한다. 소스 공개와 온라인 스튜디오 배포는 별개다.

### 이전 기능 릴리스 — renderer 0.12.0 / figgy 0.10.0

0.12.0 / 0.10.0에는 원본 데이터를 나누어 그리는 스트리밍 기능과 웹 작업 API인 `render_chart()`가 추가됐다. 렌더러가 필요한 데이터 구간을 요청하면 웹 래퍼가 원본을 공급하고 작업 순서와 진행 상태를 관리한다.

지원되는 정밀 모드의 점·실선·오차 막대 차트에서는 현재 화면에 필요한 원본 행만 차트별 GPU 캐시에 보관할 수 있다. 서로 연결된 컬럼 전체를 자동으로 상주 풀에 옮기지는 않는다. 완료된 결과는 재사용하며, 표시 범위를 넓히거나 출력 배율을 바꾸면 같은 원본을 다시 읽어 그린다. LOD나 다운샘플링으로 데이터를 줄이지 않는다.

지원 범위와 제한 사항은 [WASM.md](crates/renderer/WASM.md)에 정리했다. 브라우저 패키지는 사용할 소스 커밋에서 빌드하며, 크레이트는 crates.io에 배포하지 않는다.

모델은 로그 등고선의 매우 작은 값 처리 수정을 포함한 **0.7.2**다. 렌더러와 웹 패키지는 wgpu 30, 개발 툴체인은 Rust 1.99.0을 사용한다.

이전 renderer 0.11.0 / figgy 0.9.1 릴리스의 변경 내용:

화면에서 폭이 1픽셀보다 좁은 히스토그램 구간은 GPU가 픽셀 열별 최댓값을 골라 0까지 채운다. 외곽선 두께와 불투명도가 모두 양수이면 외곽선 색으로, 그렇지 않으면 채움색으로 영역 전체를 그린다.
원본 컬럼과 웹 API 형식은 그대로다. Rust의 `ColumnBarLayer`에는 `envelope` 필드가 추가됐으므로 구조체를 직접 생성하는 코드는 이 필드를 지정해야 한다. 수동으로 만든 레이어에 해당 자원이 없으면 `None`을 사용한다.
데모에는 구간 수 조절 슬라이더와 외곽선 표시 옵션이 있다. `npx serve crates/web -l 8142`를 실행한 뒤 `http://localhost:8142/`에서 확인할 수 있다.

renderer 0.10.0 / figgy 0.9.0 릴리스에 포함된 기능은 다음과 같다.

- **히스토그램과 행렬 데이터를 GPU에서 그린다.** `Histogram`은 구간 경계와 빈도 값을 명시적으로 받는다. `Heatmap`, `Contour`, `HeatmapContour`는 같은 행렬 격자와 색상표 설정을 사용한다. 히트맵은 셀 단위 채움과 보간을 지원하고, 등고선은 최대 1024개 레벨을 지원한다. 등고선 라벨이 놓인 곳에서는 선을 끊어 글자가 읽히도록 한다. 자동 범위 맞춤은 실제로 그려지는 셀 경계를 기준으로 한다.
- **선택한 데이터는 식별자로 추적한다.** `pick_data`는 점·히스토그램 구간·행렬 셀·등고선 레벨을 구분하는 참조를 반환한다. 이 참조를 `Config.picked_data`에 넣으면 해당 데이터를 강조할 수 있다. 히스토그램은 막대 폭, 외곽선 색·두께, 구간별 스타일을 지원한다. 등고선 라벨의 글자색·배경·숫자 형식은 선 색과 별도로 설정한다. 색상 막대는 눈금·라벨·제목·방향 반전과 크기 조절을 포함한 전체 `AxisOptions`를 제공한다.
- **범위 계산과 초기화 과정을 확인할 수 있다.** 행렬 좌표의 hi/lo 연산과 범위 계산은 GPU에서 수행한다. 축 설정에 기록하는 범위는 실제 그리기에 사용하는 값과 같다. 브라우저 초기화에서는 등고선 라벨의 너비 속성을 포함해 모든 렌더링 진입점을 검증한다. `prewarm_all_with_progress` 또는 `prewarm_all`을 호출하면 필요할 때 생성되는 렌더러 캐시를 미리 준비할 수 있다.
- **주요 GPU 자원의 할당량을 추적한다.** 풀 저장소, 업로드용 스테이징 버퍼, 이미지 출력, 피킹, 등고선 라벨 배치에 메모리 예산 검사를 적용한다. 은하수·별자리 스타일의 미리 생성한 텍스처는 일반 집계에서 제외된다. 따라서 보고값은 전체 GPU 메모리나 실제 VRAM 사용량과 다르다. 자세한 범위는 [메모리 계상과 회수](crates/renderer/ARCHITECTURE.md#메모리-계상과-회수)를 참고한다.

이 라이브러리는 공개 저장소의 소스로 제공하며 crates.io에는 배포하지 않는다. 특정 공개 Git 커밋을 사용하는 프로젝트는 버전을 바꿀 때 잠금 파일을 갱신하고 WASM 패키지를 다시 빌드해야 한다. 브라우저 API와 수명 관리는 [WASM.md](crates/renderer/WASM.md), JSON 형식은 [SCHEMA.md](crates/web/SCHEMA.md)에 설명돼 있다.
