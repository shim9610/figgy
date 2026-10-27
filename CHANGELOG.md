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
written by model 0.12 and earlier parse and re-serialize byte for byte; the
new keys (`"math"`, `"math_node"`, `"math_parent"`, `{"Plain":{}}`,
`categories`, `cross_at`, `bar_groups`, `log_ticks`, `line_offsets`,
`grid_shape`, the radar series style's `fill_opacity`, the value labels'
`offsets`, the `value_label` selection and
the `CategoryBars`, `BoxPlot`, `Radar` and `Streamlines` series kinds) are
additive.

<a id="unreleased-model"></a>

### Model (`figgy-model` 0.13.0)

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
  `out_margin` grows, never shrinks, to the labels' reach, `LABEL_GAP`, the
  title's thickness and `LABEL_GAP` again (a band that already holds them,
  or a side with neither labels nor a title, is kept); a title that does not
  clear its labels by `LABEL_GAP` inside its band gets the `offset_y` that
  puts it `LABEL_GAP` past them, and one that already clears, centred or
  offset by the host, stays; overlay axes and the colourbar's axis follow the
  same rule, the title band follows `fit_chart_title_margin`, a
  legend over the colourbar goes to the first corner that clears it, and
  each subplot is laid out as its own chart. No serialized field is added.
  Not covered: overlay axes drawn over each other, polar angle labels against
  radius labels, and labels at a corner, which run along their axes. Call it
  again after an auto-fit or a zoom: the range helpers reset a linear axis to
  `Decimal`.
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

<a id="unreleased-renderer"></a>

### Renderer (`figgy-renderer` 0.19.0)

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
  writes the `(hi, lo)` split, as `Config::from_boxplot`'s uploads and the
  WASM `register_ragged` do; `Column<f64>` writes `(value as f32, 0)`,
  exact to 2^24. Migration: the unreleased `MAX_RAGGED_VALUES` is removed.

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
- **New dependency.** `ttf-parser 0.25` (`default-features = false`,
  features `std` and `opentype-layout`) reads the MATH table and the GSUB
  `ssty` alternates; the lockfile already held it through winit.
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

<a id="unreleased-web"></a>

### Browser product (`figgy` 0.16.0)

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
- **Rust literals and matches.** `Config { .. }` adds `bar_groups: None`
  and `line_offsets: None` (a subplot's `config` likewise);
  `AxisOptions { .. }` adds `categories: None` and `cross_at: None` (an
  exhaustive destructuring names both); `PolarOptions { .. }` adds
  `grid_shape: None`; a `RadarSeriesStyle` literal that names every field
  adds `fill_opacity: None`; the three series label types add
  `offsets: Vec::new()`; `data_render::SeriesLayers` adds
  `value_labels: None`; a `DecorationElement` literal drops `followers`
  (`..Default::default()` covers the additions). Exhaustive matches add
  `DataRenderType::{CategoryBars, BoxPlot, Radar}`,
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
  default config and the released config documents (only the 0.10.0 and
  0.11.0 edited documents change, listed), a serde round trip and
  `set_chart_config`; the model unit tests pin the band rule, offsets,
  formats, the colourbar, the legend, overlays and subplots with a stub
  measure, and the web crate `autoLayout`.
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
  each primitive.
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
