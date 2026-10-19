# Configuration reference / 설정 명세

This reference describes the Rust model shared by native and WASM consumers.
The optional JSON representation is described in [SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md).

Polar coordinates (`Config::polar`, `LabelFormat::Degrees` / `Radians`), radar
charts (`RadarChart`), vector fields (`DataRenderType::Vector`), point colour
and size maps, area fills and step lines (`fill`, `fill_color`,
`interpolation`) and the `Config` fields `overlays`, `subplots`,
`active_layer` and `annotations` are on the repository's master branch and are
not in the published figgy-renderer 0.17.0 / figgy-model 0.11.0. Until the
next release, depend on the repository:

```toml
renderer = { package = "figgy-renderer", git = "https://github.com/shim9610/figgy" }
```

`Config` and the chart's series list are the single source of truth (SSOT)
for what is drawn. For precise control, edit only the desired fields in the
current `Config`.
Changing `min` / `max` directly preserves the configured tick spacing and label
format. Convenience helpers can also choose ticks or fit layout: for example,
`Chart::set_x_range` adjusts ticks on both X axes, and `Chart::with_title` can
enlarge the title band. See [direct SSOT edits and convenience helpers](API.md#direct-ssot-edits-and-convenience-helpers)
for the operation table, a range-only example and the required refresh step.

## Config struct field reference

```rust
pub struct Config {
    pub chart_area: ChartArea,           // panel pixel rect (inside the host viewport)
    pub top_x: AxisOptions,              // 4-side axes — top/right labels & titles disabled by default
    pub bottom_x: AxisOptions,
    pub left_y: AxisOptions,
    pub right_y: AxisOptions,
    pub chart_title: ChartTitleOptions,
    pub grid: GridOptions,
    pub legend: Legend,
    pub picked_points: Option<PickedPointsConfig>,
    pub picked_data: Option<DataSelectionsConfig>,
    pub colorbar: Option<ColorBarOptions>,   // the colourbar AND the chart's z scale
    pub draw_style: DrawStyle,
    pub polar: Option<PolarOptions>,         // None = rectangular; Some draws bottom_x as the angle, left_y as the radius
    pub overlays: Vec<OverlayLayer>,         // secondary right Y axes, each scaling the series it lists
    pub subplots: Vec<Subplot>,              // more charts in the same document, each with its own axes
    pub active_layer: LayerRef,              // interaction target layer; `Base` (default) = the chart's own series and decorations
    pub annotations: AnnotationLayer,        // dedicated annotation layer: visibility + items
}
```

### `ChartArea` / `Rect`
| Field | Type | Meaning |
|---|---|---|
| `x, y` | u32 | Top-left pixel position relative to the host surface |
| `width, height` | u32 | Panel pixel size. 0 → live raster fails (`InvalidChartArea`); callers should keep export chart areas non-zero too. Export's current 1 px clamp is a compatibility guard and may become an explicit error |

### `AxisOptions` (top_x / bottom_x / left_y / right_y)
| Field | Type | Meaning |
|---|---|---|
| `scale` | `AxisScale` | `Linear` or `Logarithmic` (log10) |
| `min, max` | f64 | Data-space range. For log scale, positive bounds are used as-is; a manual non-positive or non-finite `min` is guarded to `1e-12` on renderer/axis paths, and a `max` that is non-positive, non-finite or not above `min` becomes one decade above `min`. Non-positive line/scatter coordinates are omitted on the corresponding logarithmic axis; line segments touching an omitted sample form gaps rather than connecting across it. The source data and other valid samples are preserved |
| `major_spacing` | f64 | linear: data units; log: decade step (1, 2, …). When a linear spacing is non-finite, ≤ 0, too small to advance from the range ends, or would draw more than 1000 intervals across the range, the raster draws the automatic spacing for the range instead. A log step is floored to a whole number of decades and capped at 2147483647 (`i32::MAX`); a step below 1 or non-finite draws 1. Each substitution is reported by `ChartView::render_warnings` after the next refresh |
| `minor_count` | usize | Minors per major (linear). On a logarithmic axis the value is a switch: 0 hides the minors and any other value draws the eight sub-decade minors 2..9 of every decade (`Chart::set_x_range` / `set_y_range` write 8). A linear count over 99 draws 99 minors per major and is reported the same way |
| `inverted` | bool | Reverses the visual direction of this axis. Tick/grid placement, data rendering, and picking all use the same reversed mapping; `min`/`max` remain the data-space bounds |
| `label_style` | `LabelStyle` | Tick-label styling |
| `tick` | `TickVisibility` | `None / Outside / Inside / Both` |
| `title_option` | `AxisTitleOptions` | Axis title text / visibility / offset |
| `out_margin` | f32 | Outer (label + title band) pixel margin |
| `line_visible / line_color / line_width / line_style` | mixed | Axis line appearance; `line_visible` controls the axis line independently of `tick` and tick-label visibility. CPU raster strokes floor to 1 px, so sub-pixel widths do not disappear |
| `line_offset` | f32 | Detached-axis offset: shifts the axis chrome (line/ticks/labels) perpendicular to itself while the data area stays put. Layout-neutral; the drag system's axis movement lands here |
| `major_tick_length / minor_tick_length` | f32 | Tick mark length (px) |

### `LabelStyle`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | Tick-label visibility; labels draw only when both `visible` and `label_visible` are `true` |
| `color` | `Color` | Label color |
| `font_size` | f32 | px |
| `label_visible` | bool | Tick-label visibility, combined with `visible` (both must be `true`). Turning either off keeps the axis line and ticks and leaves `out_margin` unchanged |
| `label_font` | String | Font family. Empty string → bundled Liberation Sans. A family that is neither registered nor installed also falls back to it; `text_render::font_family_available(name)` tells hosts whether a name resolves. A character the face cannot draw falls back per character to registered fonts, the bundled Liberation Sans, then (native only) installed system fonts |
| `label_offset_x / y` | f32 | Fine nudge offset (px). In polar mode the angle axis reads `y` along the spoke and ignores `x`; the radius axis reads `x` along the ray and `y` across it (see `PolarOptions`) |
| `format` | `LabelFormat` | `Decimal / Power / Scientific / Timestamp / Degrees / Radians`. `Timestamp` interprets numeric values as Unix epoch time on linear axes and can use calendar-aware ticks. `Degrees` is decimal with a degree sign; `Radians` writes multiples of π (`π/2`, `3π/4`, `0.4π`) |
| `significant_digits` | u8 | Requested numeric precision (1–15 for decimal/scientific labels). Linear decimal labels share a decimal count based on range magnitude, raised only as needed to distinguish the tick interval; logarithmic labels round each value. Does not change tick positions |

Numeric labels that would overlap along an axis are drawn at a coarser label stride;
the configured ticks and grid lines remain unchanged. Precision changes do not
automatically enlarge `out_margin` or move titles: increase the explicit margin
when a deliberately long label needs more room. The same formatting rules apply
to colorbar tick labels.

`LabelFormat::Timestamp` keeps data coordinates numeric. Its default config is
UTC Unix seconds with `AutoCalendar` tick planning; use
`unit: TimestampUnit::Milliseconds` for JS timestamps and
`timezone: TimestampZone::FixedOffsetMinutes(540)` for KST, for example
`LabelFormat::Timestamp(TimestampLabelFormat { unit: TimestampUnit::Milliseconds, ..Default::default() })`
(types in `renderer::format`).
`AutoCalendar` measures tick-label text and coarsens the calendar step so labels
do not overlap. Use `Renderer::add_hilo_column` (native) or
`register_column_f64` / `update_register_column_f64` (web `Float64Array`) for
high-resolution absolute Unix timestamps; those paths preserve sub-f32 deltas
on the GPU as f32 hi/lo pairs.
The browser demo at [crates/web/timestamp-demo.html](https://github.com/shim9610/figgy/blob/master/crates/web/timestamp-demo.html)
is the quickest visual check for range changes, chart-width changes, and export
scale against that contract.

### `AxisTitleOptions` / `ChartTitleOptions`
| Field | Type | Meaning |
|---|---|---|
| `text` | `RichText` | greek / sub/super / bold/italic styled segments; `align` (`Left` default / `Center` / `Right`) places the lines of multi-line text |
| `visible` | bool | |
| `offset_x / y` | f32 | nudge |
| `top_margin` | f32 | (chart_title only) chart-title band height; the title is centred in it. The title helpers (`Chart::with_title`, the web `set_title`) grow it through `layout::fit_chart_title_margin` when the title is taller than the band; a value injected directly is kept as given |

### `GridOptions`
| Field | Type | Meaning |
|---|---|---|
| `show_major_x/y` | bool | Major grid lines |
| `major_x/y_color, _width, _style` | mixed | Major line appearance (Solid / Dash / Dot, 11 presets) |
| `show_minor_x/y` | bool | Minor grid lines |
| `minor_x/y_color, _width, _style` | mixed | Minor line appearance |

### `DrawStyle`
| Variant / JSON mode | Meaning |
|---|---|
| `Precise` / omitted or `{ "mode": "precise" }` | Default scientific renderer; serialized default omits `draw_style` |
| `Sketch` / `{ "mode": "sketch", ... }` | Hand-drawn chart-wide style. Draws line, scatter and errorbar series; bars, fields, contours and vectors are skipped (query with `draw_style_supports`). Parameter metadata comes from `DrawStyle::param_specs_for_mode("sketch")` (web: `draw_style_param_specs("sketch")`) |
| `Milkyway` / `{ "mode": "milkyway", ... }` | Astrophotograph chart-wide style. Draws line, scatter and errorbar series; bars, fields, contours and vectors are skipped (query with `draw_style_supports`). Parameter metadata comes from `DrawStyle::param_specs_for_mode("milkyway")` (web: `draw_style_param_specs("milkyway")`) |
| `Constellation` / `{ "mode": "constellation", ... }` | ScatterLine-only star chart style. Parameter metadata comes from `DrawStyle::param_specs_for_mode("constellation")` (web: `draw_style_param_specs("constellation")`) |

### `PolarOptions` (`Config::polar`)

`polar: None` (default, key absent in JSON) is the rectangular chart. `Some`
draws the same series in polar coordinates: `bottom_x` is the angle axis and
`left_y` the radius axis, so their ranges, ticks, label styles and titles
keep working (the radius may be logarithmic; the angle axis must stay linear
and non-inverted); `top_x` and `right_y` are not drawn. The data stays the
host's (angle, radius) columns. The plot is the largest circle inside the data
area that leaves one band of angle labels inside it (`Config::polar_geometry`).
[POLAR.md](POLAR.md) walks through a first polar chart, and
[examples/polar_plot.rs](examples/polar_plot.rs) is the runnable example.

| Field | Meaning |
|---|---|
| `zero_angle_degrees: f64` | Where the angle value 0 points, in screen degrees counter-clockwise from the right: 0 right (default), 90 up |
| `clockwise: bool` | Whether angle values grow clockwise on screen. Default `false` (counter-clockwise, the mathematical convention) |
| `angle_unit: AngleUnit` | `Degrees` (default) or `Radians`: how the angle column and `sector` are read |
| `sector: Option<[f64; 2]>` | The visible angular window in `angle_unit`, at most one turn; drawing is clipped at its edges. `None` (default) is the full circle, where an angle outside the axis range lands where `cos` / `sin` put it: nothing is wrapped, sorted or deduplicated |
| `inner_radius_fraction: f32` | The hole at the centre as a fraction of the outer radius, `0.0..=0.9`; `left_y.min` maps to this ring |
| `interpolation: PolarInterpolation` | `Chord` (default): a segment between two samples is straight on screen. `Arc { pieces }` (1–64): the segment is `pieces` chords between points interpolated linearly in (angle, radius), so a constant radius follows its ring; dashes and picking walk the same pieces |
| `radial_label_angle_degrees: f64` | The screen direction of the ray the radius axis line and its tick labels sit on, default 90 (up). Dragging the ray turns it |

The grid draws `*_y` fields as rings at the radius ticks and `*_x` fields as
spokes at the angle ticks. The angle axis is the outer ring with its ticks
pointing outward and its labels around it; a full-circle axis draws one spoke
and one label where two ticks coincide (0 and 360). `LabelFormat::Degrees`
(`90°`) and `LabelFormat::Radians` (`π/2`, `3π/4`, `0.4π`) format the angle
labels. A radius below `left_y.min` is not drawn (a line breaks there); on a
logarithmic radius, non-positive samples are masked like on a rectangular log
axis. Everything else a series draws is clipped to the plot: past the outer
ring (a radius above `left_y.max`, as when the radius axis is zoomed in),
inside the hole and across a sector edge a primitive is cut, the way the
data-area scissor cuts a rectangular chart, so a segment whose sample lies
outside the sector is drawn up to the edge. A marker outside the sector is
neither drawn nor picked; a line's pieces stay pickable where they are
drawn. A line fill (`LineFill::ToBaseline`)
fills to the baseline ring, the centre for a baseline at the axis start.

Validation (`Config::validate_polar`, reported as `InvalidConfig { field:
"polar" }`): finite angles, `inner_radius_fraction` in range, a sector with
`start < end` of at most one turn, arc pieces in `1..=64`, a linear and
non-inverted angle axis (use `clockwise`), and no `overlays`. Series that have
no polar form are rejected with `InvalidSeriesConfig`: histograms, heatmaps,
contours, error bars and vectors; `Arc` needs a linearly interpolated line.
Streamed sources are rejected. Annotations are placed by ratio only:
reference lines, spans and data anchors report a skip (`NoValueAxis`).
Subplots carry their own `polar`, so polar and rectangular panels mix in one
document.

Dragging follows the polar frames. The radius axis ray (`"axis_left"`) turns
around the centre: with the pointer position (`Draggable::drag_from`,
`Config::nudge_from`, what the web wrapper's `on_move` sends) it follows the
pointer's angle exactly, without one (`drag_by`, keyboard nudges) a point on the outer ring
follows the drag's component across the ray; the result lands in
`radial_label_angle_degrees`. The angle axis is the outer ring, the data
boundary, so `"axis_bottom"` has no hit region and no detach (`line_offset`
is ignored). The angle tick labels (`"tick_labels_bottom"`) move along their
own spokes: the drag's radial component at the pointer (the spoke at the zero
angle without one) accumulates in `bottom_x.label_style.label_offset_y`,
outward when positive, and `label_offset_x` has no effect. The radius tick
labels (`"tick_labels_left"`) move in the ray frame:
`left_y.label_style.label_offset_x` is along the ray (outward positive),
`label_offset_y` across it (toward the label side, counter-clockwise from the
ray, positive), so they keep their place when the ray turns. The top and
right axes are not drawn and register nothing, their titles included. Chart
title, the bottom and left axis titles, legend, data area and annotations
keep the rectangular rules.

### `LayerRef` / `AnnotationLayer`

The layer fields shared by every chart family (`Config`, `RadialChart`,
`CategoricalChart`, `BoxPlotChart`, `RadarChart`).

| Field | Meaning |
|---|---|
| `active_layer: LayerRef` | The layer that receives pointer interaction: `Base` (default, omitted from JSON), `Overlay(index)`, `Annotations`, `Subplot(index)` or `SubplotOverlay(subplot, overlay)`. Decides which layer's elements the hit map registers (`HitMap::for_active_layer`); with `Annotations` the annotation items are tested on top of the chart's own hit map and on every other layer clicks pass through them (`ChartView::hit_test_active_layer`, `BoundedAnnotationFrame::hit_test_active_layer`). Element selection (`selected`, `picked_points`, `picked_data`) lives one level below it and a layer change clears it, the data selection excepted. An `Overlay` or `Subplot` index past its list reads and is stored as `Base`, a `SubplotOverlay` whose overlay is missing as its `Subplot`; bounded charts have no overlays or subplots, so those fall back to `Base` there (`active_layer()` / `set_active_layer` on `RadialChart`, `CategoricalChart`, `BoxPlotChart` and `RadarChart`, the setter clearing `decorations.selected` on a change). |
| `annotations: AnnotationLayer { visible, items }` | The dedicated annotation layer: `items` in draw order, composited as an underlay below the data (spans) and an overlay above every decoration (everything else) but toggled, selected and edited as one unit. `visible = false` skips the two composite draws without re-rasterizing anything. The default (visible, no items) is omitted from JSON, and so is an empty `items`. |

Overlay elements are addressed as `DecorationId::Overlay { layer, inner }`
(`overlay_<n>:<inner id>`), see `OverlayLayer` below, and annotation items as
`DecorationId::Annotation(index)` (`annotation_<n>`), the item's position in
`items`.

#### `Annotation`

| Field | Type | Meaning |
|---|---|---|
| `id` | `String` | Free host label that survives reordering. Empty by default and omitted from JSON then |
| `visible` | `bool` | A hidden item is neither drawn nor registered in the hit map; its geometry is kept. `true` by default and omitted then |
| `kind` | `AnnotationKind` | One of the four variants below, externally tagged (serde's default): `"kind": {"Span": {...}}` |

| `AnnotationKind` | Fields | Where and how it is drawn |
|---|---|---|
| `ReferenceLine` | `axis: AnnotationAxis`, `value: f64`, `style: AnnotationLineStyle`, `label: Option<RichText>` | Overlay. A line across the whole plot area at `value`: vertical for `X`, horizontal for `Y`. The label sits inside the plot at the line's top (`X`) or left (`Y`) end |
| `Span` | `axis: AnnotationAxis`, `from: f64`, `to: f64`, `fill: Color` | Underlay, below the data, so a translucent fill never tints it. The band between `from` and `to`, clipped to the plot area; a band partly outside is clipped, one entirely outside is skipped |
| `Text` | `anchor: Anchor`, `text: RichText`, `box_style: Option<BoxStyle>` | Overlay. The text block's top-left corner (the frame's corner when boxed) sits at the anchor, anywhere in the panel; a block that would leave the panel is skipped |
| `Arrow` | `from: Anchor`, `to: Anchor`, `style: ArrowStyle`, `text: Option<RichText>` | Overlay. A shaft from `from` to `to` with a filled triangular head at `to`; the label is centred beside the shaft's midpoint, on its upper side |

`AnnotationAxis` is `X` (horizontal on screen) or `Y` (vertical) in every
family. Cartesian charts map them to `bottom_x` / `left_y`, the axes the data
transform draws against, with the same linear and logarithmic rules as series
(a non-positive value on a log axis has no position). Bar and box charts map
them by their direction: `Vertical` bars put the category index on `X` and the
value on `Y`, `Horizontal` bars the other way round. The category axis counts
categories, the first centred at coordinate `0`, so `-0.5..0.5` is the first
category's slot. Radial and radar charts have no value axes at all, and
neither does a polar `Config` (`polar: Some`), so only `Ratio` anchors are
placed there.

`Anchor` is either `{"Data": {"x": f64, "y": f64}}`, a coordinate on the two
document axes that follows zoom and range changes, or
`{"Ratio": {"x": f32, "y": f32}}`, a fraction of the plot area, `(0, 0)` at its
top-left and `(1, 1)` at its bottom-right corner, fixed on screen. Ratio values
outside `0..1` place a text box or arrow endpoint outside the plot area but
inside the panel. `Anchor::converted(&space, kind)` switches between the two
kinds while keeping the screen position.

The styles are `serde(default)` with these defaults:

| Type | Fields |
|---|---|
| `AnnotationLineStyle` | `color` (dark grey), `width: f32` (1.5), `style: LineStylePreset` (`Solid`) |
| `BoxStyle` | `fill` (white, alpha 0.9), `border_color` (dark grey), `border_width` (1.0), `padding` (4.0) between border and text |
| `ArrowStyle` | `line: AnnotationLineStyle`, `head_length` (10.0), `head_width` (8.0) |

Every pixel dimension (strokes, text sizes, padding, arrow heads) scales with
`Config::scaled` like the rest of the chart, so a 2× export draws the layer at
2× with the same metrics as its 1× hit map. Values and anchors do not scale.

Validation (`AnnotationLayer::validate`, part of config validation): at most
1024 items, ids of at most 160 characters without control characters, finite
colours in `0..1`, line widths, box border widths and box padding `0..64`,
arrow heads `0..256`, texts within the rich-text limits (font size 6..160, 512
segments, eight lines). Non-finite values and anchors are **not** errors: the
raster skips the item, keeps it in the document and reports
`RenderWarning::AnnotationSkipped { index, reason }` with `NonFinite`,
`NoValueAxis` (a data anchor, reference line or span on a radial, radar or
polar chart), `OutOfRange` (no position on its axis or outside the panel) or
`EmptyText`. Annotations never
take part in the data-area margin computation (a new text box does not shrink
the plot) and the automatic label placer never nudges them.

### `OverlayLayer` (`Config::overlays`)

An overlay layer is a second Y axis drawn on the right side of the shared data
area, scaling the series it lists. `overlays` is empty (key absent in JSON) by
default; a layered document is still one
`Config`: the chart area, both X axes, the title, the grid, the merged legend,
the colourbar and `draw_style` exist once, on the parent, and only the fields
below exist per layer. Bounded charts (radial, categorical, box plot, radar) have
no overlays, and nonresident streaming rejects a document that has any.

| Field | Type | Meaning |
|---|---|---|
| `y_axis` | `AxisOptions` | The overlay's Y axis, drawn on the right. `min` / `max` / `scale` / `inverted` map the overlay's series to pixels; `out_margin + major_tick_length` is its band in the shared right margin, which is the widest band of the parent `right_y` and every overlay (`Config::margins`). Defaults to the default left-axis options (labels and line visible). |
| `series` | `Vec<String>` | Ids of the series this layer scales, from the chart's single series list. An id may belong to at most one overlay (`Config::validate_overlays`); an id with no declaration is ignored; a series that no overlay or subplot lists belongs to the base layer. Omitted from JSON when empty. |
| `legend` | `Option<Legend>` | A legend drawn in this layer's panel. `None` (omitted) keeps the single merged legend on the parent for every layer's series. |

The renderer draws the document as one internal panel per layer through
`Config::expand()`: the base panel is the parent without its overlays; each
overlay panel keeps the parent's X range for its transform but draws no X axis,
grid, title or colourbar, carries the overlay's range on a hidden left axis and
draws the overlay's `y_axis` on the right. Every panel's margins are lifted to
the group's, so `data_area()` is the same rectangle for all of them and for
the parent; `Expansion { panels, data_area }` returns them. Expansion is pure,
copies configuration only and never serializes the panels. A data-area edge
drag or `fit_to_data_area` writes the group band through the parent's
`right_y.out_margin` and pulls every overlay band inside it. `Config::scaled`
scales the overlay axes and legends like the parent's. The parent's own
`right_y` stays as written: hide its line and ticks when an overlay should have
the right band to itself.

`active_layer` addresses one layer for interaction (`Config::active_layer()`
applies the out-of-range fallback, `set_active_layer` stores it) and
`Config::layer_of_series(id)` answers which layer scales a series. See the
[interaction guide](INTERACTION.md#overlay-layers) for the hit map, selection
and export behaviour. `Chart::set_overlay_y_range(layer, min, max)` sets an
overlay's range and derives its ticks, and
[examples/overlay_axes.rs](examples/overlay_axes.rs) draws twin Y axes.

### `Subplot` (`Config::subplots`)

A subplot is one more chart drawn in the same document. `subplots` is empty
(key absent in JSON) by default. Each
subplot is a complete Cartesian chart with its own `chart_area` (in the
parent's surface coordinates), all four axes, title, grid, legend, colourbar,
style and secondary Y axes; nothing is shared with the parent or another
subplot unless a link says so. The document's own `Config` stays the first
chart, so a 2×2 grid is the parent plus three subplots.

| Field | Type | Meaning |
|---|---|---|
| `config` | `Config` | The subplot's chart. Its `overlays` are secondary Y axes inside it (one nesting level: its own `subplots` must be empty). Its `annotations` must stay default, since the document has one annotation layer, on the parent; its `active_layer` is ignored. |
| `series` | `Vec<String>` | Ids of the series this subplot draws, from the document's single series list, including the ones its own overlays scale. An id belongs to at most one subplot and to no parent overlay (`Config::validate_subplots`); ids with no declaration are ignored. Omitted from JSON when empty. |
| `link_x` | `Option<CellRef>` | `None` (default, omitted): the X axis is the subplot's own. `Some(CellRef::Parent)` or `Some(CellRef::Subplot(j))` with `j` below this subplot's index: `bottom_x` follows the target's `bottom_x` range, scale, direction and tick spacing (`major_spacing`, `minor_count`). Line, label and title style stay the subplot's own. |
| `link_y` | `Option<CellRef>` | The same for `left_y`. |

A link group has one value: `Config::subplot_config(i)` resolves subplot `i`
with its linked axes copied from the root of their chain, and
`Config::sync_subplot_links(previous)` stores the linked fields equal to their
root. Given the document before an edit, it first carries an edit of any
member to the root, so writing a new range to a linked axis moves the whole
group; an edit of the root itself wins. `Renderer` runs it when a chart is
registered and inside every config, state and view-state replacement
(`register_chart`, `set_chart_config`, `set_chart_state`,
`set_chart_view_state`), so a linked edit lands on every chart in one
`&mut self` call. `Chart::set_subplot_x_range` /
`set_subplot_y_range` and `auto_fit_subplot_{x,y}_{extent,union}` write the
root of a linked axis directly.

The renderer expands every subplot by the same rules as the parent
(`Config::expand`, after the parent's panels, `Config::panel_layers` gives
their order), each in its own area with its own data area. The document covers
the union of every chart area (`Config::document_area`): the document export
renders it, a display fits it to the surface and moves every chart with the
parent (`Config::translate_areas`, `display_config_for_surface`,
`place_document`). `Config::scaled` scales each subplot like the parent.
[INTERACTION.md](INTERACTION.md#subplots) covers selection and dragging, and
[examples/subplots.rs](examples/subplots.rs) draws a 2×2 grid with a linked X
axis.

### `Legend`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | |
| `content` | `RichText` | The whole legend as **one rich document**: `'\n'` segments break lines, symbols are inline segments (glyph char + per-segment `color` override) — breaks, symbol positions, and mid-text symbols are all explicit in the SSoT. `font` / `font_size` are live at draw time |
| `corner` | `LegendCorner` | `TopLeft / TopRight / BottomLeft / BottomRight` |
| `padding` | f32 | Legend box internal padding. Corner placement uses the fixed data-area inset plus `offset_x / offset_y` |
| `offset_x, offset_y` | f32 | Screen-pixel offset from the corner placement. It does not change the margins; a legend drag accumulates here |
| `bg_color, border_color` | `Color` | Box background / border |

Symbols are **fixed-width field segments** (`field_em`): every form spans
exactly `SYMBOL_FIELD_EM` (2.0 em × font size) regardless of shape — a line
mark is a drawn rule (`rule: true`) filling the whole field, a scatter mark
is the shape glyph (`● ■ ▲ …`) centered in it, and line+scatter is
rule–glyph–rule summing to the same width. Dashed/dotted line styles are
carried by `rule_dash` on rule segments, so legend marks reflect
`LineStylePreset` as well as color and shape. Auto-built entries are
`symbol + ' ' + '\t' + label`, so labels also align via the tab column.
Composition helpers: `symbol_segments(kind, color)`,
`series_symbol_segments(cfg)`, `append_legend_entry(content, symbol, label)`.

### `PickedPointsConfig`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | Enables/disables the overlay when `picked_points` is present |
| `refs` | `Vec<PickedPointRef>` | Picked data references: `series_id`, optional `source_id`, and `point_index`. The overlay stores provenance, not copied coordinates |
| `ring_color` | `Color` | Highlight ring color |
| `ring_width_px` | f32 | Ring stroke width in pixels |
| `radius_extra_px` | f32 | Extra radius added around the source marker |

Missing `picked_points` / JSON `null` means no picked-point overlay. JSON `{}` is accepted as the default overlay config (`visible: true`, empty refs, gold ring, 2 px stroke, +3 px radius), so hosts can turn the overlay on and then fill `refs`.
The overlay ring follows the picked scatter marker radius, including per-point style mapping; for line-only picks it uses `radius_extra_px` around the snapped endpoint.

### `DataSelectionsConfig`

`Config.picked_data` holds `visible` and `refs`, a list of `PickedDataRef`
identities tagged by `"kind"` in JSON (`"point"`, `"histogram_bin"`,
`"matrix_cell"`, `"contour_level"`): `Point`,
`HistogramBin`, `MatrixCell`, or `ContourLevel`. Every ref has `series_id` and
optional `source_id`; its kind then carries `point_index`, `bin_index`, canonical
`x_index/y_index`, or `level_index + x_index/y_index`. Visual policy is
`highlight_color`, `outline_width_px`, `point_radius_extra_px`, and
`contour_width_extra_px`. No ref stores coordinates, bar bounds, or contour
segments. The overlay resolves current geometry from the same GPU resources as
the normal draw, and stale/out-of-range indices draw nothing. JSON `null`
clears it and `{}` selects the default empty gold overlay (`visible: true`,
2 px outline, +3 px point radius, +2 px contour width).

### `ColorBarOptions`
| Field | Type | Meaning |
|---|---|---|
| `visible` | bool | `false` draws nothing **and reserves no band** — the space returns to the data area. Field series still render |
| `side` | `Side` | `Left`/`Right` = vertical bar, `Top`/`Bottom` = horizontal. This alone decides the orientation |
| `thickness_px` | f32 | The strip's short dimension |
| `gap_px` | f32 | Space between the data area and the strip |
| `length_frac` | f32 | Strip length as a fraction of the data area's length along that side; must be in `(0, 1]` |
| `align` | `BarAlign` | `Start` / `Center` / `End` along that side — the discrete half of the anchor |
| `offset_x`, `offset_y` | f32 | Free offset from that anchor, in screen pixels. **Margin-noncontributing**, the same contract as `Legend::offset_{x,y}` and the title / label offsets, so nudging the bar moves it without reflowing the data area. This is where a drag accumulates |
| `colormap` | `ColorMap` | `Viridis` / `Magma` / `Turbo` / `GrayScale` / `RdBu` / `Custom { stops }` |
| `nan_color` | `Color` | Colour for z the ramp cannot place: NaN, and non-positive z on a log colourbar. Fully transparent by default |
| `border_color`, `border_width` | `Color`, f32 | Strip border |
| `axis` | `AxisOptions` | **The z axis — the single source of the z range** |

`axis` is a full `AxisOptions`: `scale`
(including `Logarithmic`), `min`/`max`, `major_spacing`, `minor_count`,
`label_style` (including `LabelFormat::Power`), `tick`, and `title_option` mean
exactly what they mean on a chart axis, and tick generation, label formatting,
and log handling are the same code rather than a parallel implementation.

Consequences:

- A `Heatmap` / `Contour` / `HeatmapContour` series **requires** this to be
  `Some`. Without it there is no z range and no colormap anywhere, so there is
  no value to draw — the renderer rejects the series rather than inventing one.
- There is therefore **one z scale per chart**; several heatmaps share it.
- The band is `gap_px + thickness_px + axis.out_margin + axis.major_tick_length`
  on its own side. `fit_to_data_area` / `resize_chart_area_scaled` may reclaim
  `axis.out_margin` (label space, like an axis') but never the strip itself, so
  a colourbar does not get thinner with the window.
- Two defaults differ from a chart axis: `line_visible: false` (the strip's
  border is that edge) and `tick: Outside` (ticks sit in the label margin
  instead of over the colours).
- The band is the **outermost** part of its side's margin: from the chart edge
  inward it is the bar's label margin, its ticks, the strip, `gap_px`, and only
  then that side's axis band. Axis tick labels are drawn from the data area
  outward and cannot move, so a strip placed next to the data area would be
  drawn on top of them.
- The bar is drawn on the CPU in the decoration layer, with no GPU pipeline. Its
  ticks, labels, and title go through the same helpers the four chart axes use,
  so a logarithmic colourbar gets decade ticks and 10ⁿ labels from the code that
  already does that for a logarithmic axis. `axis.tick` controls
  inside/outside/both, `axis.inverted` controls the min→max screen direction,
  and ticks are painted with the same `line_color` / `line_width` /
  `line_style` as its axis line.
- It is a **selectable, draggable, resizable** element like the rest of the
  chrome: hit-test id `"colorbar"`, a blue selection box, and — with the data
  area, the only two that have them — the eight resize handles. A drag lands in
  `offset_{x,y}`; a handle drives `thickness_px` or `length_frac` depending on
  the *bar's* orientation, which nudge resolves because a handle only knows
  screen directions. Its details are independent foreground targets:
  `"colorbar_axis"`, `"colorbar_tick_labels"`, and `"colorbar_title"`.
  Dragging them updates `axis.line_offset`, label offsets, and title offsets
  respectively. All three derive from the painted strip rect; a shortened,
  aligned, moved, or resized strip therefore keeps its title and hit geometry
  attached.
- `ColorBarOptions::normalized_z(z) -> Option<f32>` is the one z→colour
  normalization (`color_for_z` applies it): clamped to `[0, 1]`, `None` for NaN,
  for non-positive z on a logarithmic bar, and for a degenerate range — those
  draw as `nan_color` rather than clamping to an endpoint, because "missing" and
  "smallest" are different facts. `axis.inverted` is not applied: it moves where
  a value is drawn, not which colour it has.

### `data_config` — declarative series schema (the active API)

Series are declared via `data_config::SeriesConfig`. `Renderer::prepare` (which `Renderer::paint` calls) matches on `render_type` and builds the layers that variant draws (line, scatter, errorbar, bar, field, contour or vector); colors, widths, and shapes come from the matching sub-style.

| Type | Fields | Role |
|---|---|---|
| `SeriesConfig` | `series_id, source_id?, label, x_column: ColumnId, y_column: ColumnId, render_type` | Full series declaration. `source_id` is optional host provenance for picking; `x_column / y_column` are renderer-registered ids, resident in the pool or backed by a replayable nonresident source. In the web editing flow, `legend.content` is the live label authority; ordinary series edits update recognized legend symbols only and preserve user text. `SeriesConfig.label` becomes authoritative only for an explicit `reset_legend_from_series_labels()` rebuild |
| `DataRenderType` | enum, 14 variants | One independent draw path per variant; each variant carries exactly the sub-styles it uses |
| `ErrorRef` | `Symmetric { column }` or `Asymmetric { lower, upper }` | Errorbar column reference. Symmetric = ±σ, Asymmetric = lower/upper split |
| `DataLineStyleConfig` | `line_style, line_color, line_width, fill?, fill_color?, interpolation` | Line appearance. `fill` adds an area under the line (`ToBaseline(y)`) or between this line and another series (`ToSeries(series_id)`); `fill_color` defaults to `line_color` at a quarter of its alpha; `interpolation` is `Linear` (default), `StepPre`, `StepPost` or `StepMid`. See [Area fill and step lines](#area-fill-and-step-lines) |
| `DataScatterStyleConfig` | `point_color, point_shape, point_size, point_style_table?, point_style_index_column?, point_style_overrides?, point_color_map?, point_size_map?` | Point appearance. The optional style map applies only to precise scatter; each table/override slot can replace color, shape, size, or any subset. `point_color_map` colours each point by a value column through `Config.colorbar`; `point_size_map` sizes each point by a value column over an explicit domain. See [Point colour and size maps](#point-colour-and-size-maps) |
| `DataErrorBarStyleConfig` | `error_bar_color, _width, _cap_size, cap_width, error_bar_style_table?, error_bar_style_index_column?, error_bar_style_overrides?` | Errorbar appearance. The optional style map applies only to precise errorbars; each table/override slot can replace color, stem width, cap half-size, cap width, or any subset |
| `DataBarStyleConfig` | `fill_color, border_color, border_width, baseline, gap_px, width_ratio, orientation, bar_style_overrides?` | Histogram appearance. `width_ratio` controls the centred fraction of each bin; sparse overrides replace fill, outline, gap, or width for selected bin indices |
| `ScatterShape` | enum, 26 variants | Circle / Square / Triangle directions / Diamond / Cross / Plus / Pentagon / Hexagon / Octagon / Star + filled variants |
| `MatrixRef` | `columns, orientation, grid_layout` | The grid of a `Heatmap` / `Contour` / `HeatmapContour` series or a grid vector: registered column ids in grid order. `orientation` is `ColumnsAreX` (each column is one x position holding its y values) or `ColumnsAreY`; `grid_layout` is `Edges` (the coordinate columns hold n + 1 cell boundaries) or `Centers` (n cell centres). See [Matrix fields and contours](#matrix-fields-and-contours) |
| `FieldFillConfig` | `mode, shading, opacity` | Heatmap fill. `mode` is `Continuous` or `Bands` (one colour per interval between the contour levels); `shading` is `Flat` (one colour per cell) or `Interpolated` (bilinear across the cell corners); `opacity` multiplies the colormap alpha |
| `ContourConfig` | `levels, line, per_level_color?, labels?` | Contour lines. `levels` is an explicit list in data units (at most 1024); `contour.set_nice_levels(&colorbar.axis, n)` writes about `n` round levels strictly inside the colourbar range (`ContourConfig::nice_levels` returns them without writing), and `contour.set_colormap_colors(&colorbar)` fills `per_level_color` from the ramp |
| `ContourLabelConfig` | `visible, font_size, color, format, significant_digits, spacing_px, anchors, bg_color?, bg_padding_px` | Inline level labels. `spacing_px` must be finite and positive; empty `anchors` means automatic placement |

**The 14 `DataRenderType` variants**:

| Variant | Sub-styles used | Meaning |
|---|---|---|
| `Line { line }` | line | Line only |
| `Scatter { scatter }` | scatter | Points only |
| `ScatterLine { scatter, line }` | both | Points + connecting line |
| `ScatterErrorbarX { scatter, err_x, err_style }` | scatter + errorbar | Points + X errorbars |
| `ScatterErrorbarY { scatter, err_y, err_style }` | scatter + errorbar | Points + Y errorbars |
| `ScatterErrorbarXY { scatter, err_x, err_y, err_style }` | scatter + errorbar | Points + X/Y errorbars |
| `LineScatterErrorbarX / Y / XY` | line + scatter + errorbar | The above + connecting line |
| `Histogram { bar }` | bar | Bars from host-binned `(edges, counts)`. `bar.orientation` alone decides which column is which: `Vertical` = `x_column` edges / `y_column` counts, `Horizontal` the reverse. The `edges = counts + 1` length relation is never used to guess |
| `Heatmap { matrix, fill }` | fill | Filled field only |
| `Contour { matrix, contour }` | contour | Contour lines only |
| `HeatmapContour { matrix, fill, contour }` | fill + contour | Filled field with lines over it |
| `Vector { components, style }` | vector | An arrow per sample: `components` is a (u, v) column pair aligned with the x / y anchors (`Points`) or two matrices over the coordinate columns (`Grid`); `style` is the arrow's colour, sizes, pivot, scale and optional colour map. See [Vector fields](#vector-fields) |

For a runnable vertical/horizontal example and atomic direction changes, see
[Histogram orientation](HISTOGRAM.md).

#### Point colour and size maps

A scatter series (every `DataRenderType` that draws markers) can map two ordinary
value columns onto its points. Both fields are optional and additive: documents
without them parse and serialize byte-identically, and a series with either one
draws through the precise mapped scatter path, like a series with a style table.

- `point_color_map: Option<PointColorMap { column }>` colours each point by its
  value in `column` through `Config.colorbar`, exactly as a heatmap cell is
  coloured: the colourbar owns the range, scale, ramp and `nan_color`, and the
  chart keeps one z scale. A colour-mapped series without a colourbar is rejected
  rather than given an invented range. A non-finite value, or a non-positive value
  on a logarithmic colourbar, draws in `nan_color`.
  `Chart::auto_fit_colorbar_union(pool, ids, padding)` fits the shared range
  over the column ids it is given, matrix and colour-map columns alike; the web
  `auto_fit_colorbar` collects every matrix, point colour-map and vector
  colour-map column of the chart itself.
- `point_size_map: Option<PointSizeMap { column, domain_min, domain_max, min_px, max_px, scale }>`
  maps `domain_min..domain_max` onto the marker radius `min_px..max_px` (logical
  pixels, the unit of `point_size`, scaled on export like it). `scale` is `Area`
  (default: the value is proportional to marker area, which is how bubble sizes
  are read) or `Radius`. Values outside the domain clamp to the ends; a non-finite
  value hides the point and removes it from picking. The domain is explicit and
  never inferred from the column — `PointSizeMap::fit_domain(min, max)` writes it
  from statistics the host already has, the way `auto_ticks` writes tick spacing —
  so appending a point never resizes the others. `PointSizeMap::radius_px(value)`
  is the formula; the shader and the CPU picker compute the same radius.
- Precedence per channel is sparse override > value map > style table slot >
  series base. Shape still comes from the table, overrides or base, so a
  categorical shape column combines with a continuous colour column. An override
  that sets `point_size` makes a hidden point visible again.
- The value columns are index-aligned with the series' x/y columns and must cover
  every point; a shorter column is a prepare error. They are enumerated like every
  other series column, so removing one invalidates the series, and picking
  (`Renderer::pick_chart`, web `pick_point`), the picked ring, the web
  `selection_details` (`color_value` / `size_value`) and PNG export all follow
  the mapped colour and radius. Streamed sources are rejected;
  the maps require GPU-resident columns. Stylized draw styles (Sketch, Milkyway,
  Constellation) ignore the maps, as they ignore the style table.

The GPU reads the value columns from the column pool on every draw and derives
colour and radius in the shader, so an upserted value column restyles the
points on the next draw.
`crates/renderer/examples/value_maps.rs` shows both maps.

<a id="vector-fields"></a>

#### Vector fields

`DataRenderType::Vector { components, style }` draws one arrow per sample.
`components` names where the (u, v) components come from; the series'
`x_column` / `y_column` stay the anchors, as for every other render type:

- `VectorComponents::Points { u, v }`: two value columns index-aligned with the
  anchors, one arrow per index. The count is the shortest of the four columns.
- `VectorComponents::Grid { u, v }`: two `MatrixRef`s declared exactly like a
  heatmap's matrix over the series' coordinate columns, with the same
  `orientation` and `grid_layout` (a mismatch is rejected). Under `Centers` the
  arrow sits on the coordinate, under `Edges` on the cell centre (the data-space
  midpoint of two edges, which on a logarithmic axis is not the screen
  midpoint). The drawn extent is the smallest the grids agree on, reported as
  truncated like a short heatmap matrix. A vector grid can share its coordinate
  columns with a heatmap or contour on the same grid; a sparser quiver over a
  fine field uses coarser coordinate columns of its own.

`VectorStyle` is the arrow:

- `color`, `width_px` (shaft), `head_length_px`, `head_width_px`: logical
  pixels, scaled on export like `point_size`. A short arrow scales its head down
  to its own length rather than growing a head longer than the shaft.
- `scale`: `VectorScale::Pixels { px_per_unit }` draws `px_per_unit * |(u, v)|`
  pixels, the direction following the axes' pixel-per-unit ratio (so a (1, 1)
  vector leans with the aspect of the data area). `VectorScale::Data { factor }`
  puts the tip at `anchor + factor * (u, v)` in data space, through the axis
  transform: the arrow follows an inverted or logarithmic axis, and its tips
  join the auto-fit extent. The scale must be finite and non-zero;
  `VectorStyle::fit_scale(u_bounds, v_bounds, target_px_or_units)` writes it
  from statistics the host already has, the way `PointSizeMap::fit_domain`
  writes a domain, so appending a sample never rescales the others.
- `pivot`: `Tail` (default, the anchor is the tail), `Middle` or `Tip`.
- `min_length_px`: an arrow shorter than this (after scaling) is not drawn;
  a zero vector, a non-finite component or a non-finite anchor draws nothing.
- `color_map: Option<VectorColorMap>`: `Column(id)` for the points form, `Grid(MatrixRef)`
  for the grid form (the form must match). The value colours the arrow through
  `Config.colorbar`, exactly as a heatmap cell or a `point_color_map` marker is
  coloured; a colour-mapped vector without a colourbar is rejected.

Interaction follows the form. A point vector is picked at its anchor like a
marker, with half the wider of head and shaft as its radius; the picked ring,
the web `step_selection` and `selection_details` (`u`, `v`, `color_value`) and
PNG export follow. A grid vector is picked and highlighted by cell on its `u` grid, like a
heatmap cell. Auto fit (`begin_series_fit_extent`, the web extent requests)
covers the anchors, plus the tips of a data-scaled point vector; a grid's
data-scaled tips are not part of the fit. Streamed sources are rejected; the
components require GPU-resident columns. Stylized draw styles (Sketch,
Milkyway, Constellation) draw no vectors (`draw_style_supports` returns
`false`). The legend symbol is a rule with an arrowhead in the series colour.

The shader is assembled from fragments (Cartesian projection, input form, arrow
glyph, entry; [src/data_render/SHADER_COMMON.md](src/data_render/SHADER_COMMON.md)
§5.3) and reads the component columns from the column pool on every draw.
`crates/renderer/examples/vector_field.rs` shows both forms.

#### Area fill and step lines

Every render type that draws a line (`Line`, `ScatterLine` and the
`LineScatterErrorbar*` variants) reads three optional line settings. All of
them are omitted from JSON at their defaults, so existing documents parse and
serialize unchanged.

- `fill: Option<LineFill>` paints the area between the line and a reference.
  `LineFill::ToBaseline(y)` fills to a constant data value (`0.0` is the usual
  area chart); `LineFill::ToSeries(series_id)` fills between this line and the
  referenced series' line, index-aligned the way an errorbar reads its extra
  columns. Fill between requires both series to share the **same x column**;
  a different x column, an unknown series or a self reference is an explicit
  `InvalidSeriesConfig` error, never a resampled fallback. The referenced
  series' own interpolation is not consulted: the fill follows this line's.
- `fill_color: Option<Color>` is the fill's colour. `None` derives it from
  `line_color` with the alpha multiplied by
  `DataLineStyleConfig::DEFAULT_FILL_ALPHA` (0.25);
  `effective_fill_color()` returns the value actually painted.
- `interpolation: LineInterpolation` connects consecutive samples with
  straight segments (`Linear`), or with axis-aligned step pieces: `StepPre`
  rises at `x[i]` to `y[i + 1]` and runs to `x[i + 1]`; `StepPost` runs at
  `y[i]` to `x[i + 1]` and rises there; `StepMid` runs, rises at the
  **data-space** midpoint of the two x values (not the pixel midpoint, which
  differs on a logarithmic axis) and runs again. The dash phase, the arc-length
  scan behind it and both pickers follow the stepped path, so a dashed step
  line keeps its pattern around every corner and a pick on a riser resolves to
  the nearer of the two source indices by position along the path.

Draw order inside a panel is "every series' fill, then every series' body,
then every picked highlight", each loop in SSOT series order with no
automatic sorting. Another series' fill therefore never covers this series'
line, and fills composite over each other in declaration order. The fill is a
property of the line, not a hit target: `Renderer::pick_chart` /
`pick_chart_data` (web `pick_point` / `pick_data`) and the selection overlay
keep working on the line's samples, and selecting the series selects its fill. The legend mark of a filled series carries a band of
the fill colour under its rule, inside the same fixed-width symbol field.

Constraints:

- Fills assume an x-monotonic series. Where x doubles back, the trapezoids
  overlap and a translucent fill blends twice.
- Both forms need GPU-resident columns. A streamed (nonresident) series with a
  fill or a step mode is rejected by `inspect_streaming_capability` and the
  streaming prepare path with an explicit error; chunked streams cannot
  guarantee the index alignment a fill between two series needs.
- A gap in the line (a non-finite sample, or a nonpositive sample on a
  logarithmic axis) breaks the fill exactly where it breaks the line.
- On a logarithmic y axis a baseline at or below zero has no position. It is
  clamped to the axis minimum instead of masking the whole area — an explicit
  exception to the rule that nonpositive values on a logarithmic axis are
  omitted.
- A fill is drawn whenever the draw style draws its line (Constellation draws
  only `ScatterLine` series), always with the precise geometry; a sketch,
  milkyway or constellation line keeps its own stroke on top.

[examples/area_step.rs](examples/area_step.rs) draws a forecast with bands
filled between series, a step line filled to a baseline and the three step
modes.

#### Histogram bars

Histogram width is resolved in two stages: `width_ratio` keeps a centred
`0..=1` fraction of the bin, then `gap_px` removes a fixed screen-space amount.
The gap is capped to leave at least one pixel for wider positive-width bars.
Bins whose original projected width is below one pixel use a GPU envelope:
each overlapping pixel column takes the maximum bin value and is filled to
zero (clipped to the visible axis range). Horizontal histograms use pixel rows.
The winning bin supplies the stroke colour for the entire area when its stroke
has positive width and alpha; otherwise it supplies the fill colour. Equal maxima use the first
bin. Gap and positive width ratios do not cut holes in this envelope;
`width_ratio = 0` still hides the bin. Original columns are unchanged.
`border_width = 0` disables the outline; otherwise `border_color` and
`border_width` control it. `bar_style_overrides` is a sparse declaration-order
list keyed by `index`; each record may independently replace `fill_color`,
`border_color`, `border_width`, `gap_px`, or `width_ratio`. Baseline and
orientation stay series-wide. Rendering, typed picking, and selected-bin
outlines all resolve the same final bar rectangle.

#### Matrix fields and contours

The three matrix variants declare their grid as `MatrixRef { columns,
orientation, grid_layout }` — a bundle of registered column ids and nothing
else. There is no separate matrix container: resident drawing reads those columns
from the pool, while supported nonresident Heatmap drawing replays their registered
sources in bounded ranges.
`grid_layout` says whether the coordinate
columns are cell `Edges` (n + 1) or `Centers` (n); it is never inferred from the
lengths. A declaration that does not line up with the data is **not an error** —
the smallest common extent is drawn and the truncation is reported.

<!-- contour-contract: scope=config-en max-levels=1024 -->
`ContourConfig.levels` is always an explicit list in data units (there is no
"auto" variant — a level set inferred at draw time is a value the config does not
contain). Its accepted length is `0..=1024`; 1025 or more is an error, and no
level is silently truncated. On a cache miss the renderer keeps the original
list and declaration order, while building a lookup copy from consecutive
32-level blocks sorted by value. Each fragment searches at most 32 blocks and
runs coverage math only for reachable candidates; if all 1024 levels actually
cross one cell, all 1024 are composited in declaration order. `per_level_color:
None` means every level uses `line.line_color`;
colours are not derived from the colormap. The lines themselves are drawn as the
level set of the field's own bilinear interpolation, from its analytic gradient —
so a band boundary and the line over it come out of one computation. Stroke
distance is the quadratic crossing obtained by restricting the current cell's
bilinear field to the current gradient-normal line. It is not a global shortest
distance to the whole piecewise-bilinear contour.

Non-finite levels have explicit uploaded-f32 semantics. Contour lines exclude
NaN and both infinities. For `FillMode::Bands`, the numerator counts every
`-Infinity` plus each finite level less than or equal to z, while the denominator
keeps the full declared level count:
`t=(negative_infinity_count + finite_le_z + 0.5)/(declared_level_count + 1)`.
NaN and `+Infinity` therefore affect only the denominator.

`ContourLabelConfig.anchors` is an **override**. Empty is the normal case: the GPU
places the labels itself, seeding a lattice at `spacing_px` over the data area and
projecting each seed onto its level's isoline. A projected seed is kept only if the
level is actually crossed inside the cell it lands in (the cell's corner range
contains the level, the same condition under which the stroke draws there), so a
shallow extremum that merely comes close to a level is never labelled. Normal
selection targets
`spacing_px` separation, but the per-level fallback may keep a closer candidate
rather than omit a level. `spacing_px` must always be finite and greater than zero,
including for hidden labels and explicit overrides. Automatic and explicit
placement share a 1024-label capacity. Explicit anchors whose `level_index` is
invalid are discarded, and only the first 1024 valid anchors are retained in
input order. If that resolved list is empty, automatic placement runs; otherwise
the resolved list overrides it. Only automatic placement multiplies spacing by
the frame/export scale; export validates that product after clamping the scale.
The atlas must also fit the adapter's texture-dimension limit. Invalid spacing,
scale-product overflow, or an oversized atlas fails before publishing renderer
state, so the previous chart and GPU resources remain active. An anchor is data
coordinates plus a data-space tangent, so a zoom or pan only re-projects it.
`ContourLabelConfig.color` owns the text colour independently of the line and
`per_level_color`; changing a ramp never recolours the typography. Decimal text
uses the contour-level interval (falling back to the colourbar interval), never
an x-axis interval, and `significant_digits` remains effective without allowing
adjacent levels to collapse to one string. The contour fragment reads the same
selected-anchor buffer as the label draw and omits stroke coverage inside each
label rectangle. `bg_padding_px` pads that real line gap whether `bg_color` is
opaque or absent. A `bg_color` box is antialiased at any rotation: the label quad
extends past the box into the atlas cell's transparent gutter, so the box edge is
filtered like the glyphs inside it rather than rasterized as the quad edge.

#### Draw info, fitting and series styles

`Renderer::series_draw_info(chart, series_id) -> Result<SeriesDrawInfo>` is the
series-common window onto what actually drew: `drawn_count`, a matrix' `cols` /
`rows`, and `truncated`. Column lengths are facts that arrive with the data, not
SSoT, so a mismatch never errors and never stops the draw — the smallest common
extent is drawn and reported here. That is how a host learns its 11-edge /
9-count histogram drew 9 bars, and it reports the `min(x, y)` truncation of
lines and scatters through the same call.

Histogram, Heatmap, Contour and HeatmapContour series do not go through the
point picker (`Renderer::pick_chart`, web `pick_point`), which handles line,
scatter and point-vector series. Histograms fit from their uploaded edge/value
metadata. Matrix fields use the GPU fit engine in a distinct field mode: the
CPU supplies only the resolved cell counts, and the GPU reads the same coordinate-pair pool as the field shader and
applies the same `Edges`/`Centers` plus cell/sample-lattice rule. Thus contour and
interpolated-field auto-fit stops at the actual sample endpoints (edge-coordinate
midpoints for `Edges`), while a flat field fits its actual cell boundaries. The
paired reducer works on index-aligned `(x[i], y[i])`
pairs, which a histogram (`edges` is one longer) and a grid (two independent
coordinate axes) are not. `Renderer::pick_chart_data` (web `pick_data`) picks
them through bar/field shader entries; fitting follows the histogram/field
rules above.

**`Renderer::create_style_for_series(cfg)`** extracts color/width/shape from `cfg.render_type`'s sub-styles and builds a GPU `ChartStyle` for screen paint. It returns `Result` and rejects a style before allocating when its four uniforms and optional style-map buffers exceed the remaining renderer GPU budget; those bytes remain accounted for while a prepared frame still holds the style bindings. For export, `create_style_for_series_scaled(cfg, scale)` scales pixel widths only and applies the same admission check.

**Single-direction errorbar** (`ScatterErrorbarY` etc.): direction presence is encoded in `PrimitiveStyle::primitive_flags` (Y=bit 0, X=bit 1). The inactive vertex slots reuse the already-bound anchor column and are collapsed before their error attributes are read, so prepare/export creates no hidden filler column and no host-maintained metadata. A real zero error remains a present, zero-length errorbar rather than being mistaken for an absent direction. (Symmetric variants reuse the same error column for lo/hi.)

### `Config::scaled(scale)` / `Config::scale_in_place(s)`
Multiplies every pixel-based dim by `scale`. `min/max/major_spacing`, scale enum, and colors are untouched. Used for resolution-invariant high-DPI export.

### Default builder — `renderer::default::default_config()`
- bottom_x / left_y: axis line + ticks + labels + title enabled, text starts as empty segments.
- top_x / right_y: axis line + ticks enabled, labels + title disabled, `out_margin = 8` (narrow gap).
- chart_title: visible, `top_margin = 32`, text empty.
- grid: major only, light gray.
- legend: disabled (top-right corner, 8 px padding).
- axis ranges: 0..1 placeholders with `major_spacing = 0.2`, `minor_count = 4`
  and inside ticks; replace them with `Chart::set_x_range` / `set_y_range` or
  `auto_fit_*`.
- chart_area: 1000 × 800 at (0, 0); resize it to the host viewport.
- colorbar: `None`. A field series, a point colour map or a vector colour map
  needs one: `renderer::default::default_colorbar_options()` returns a visible
  right-hand Viridis bar (18 px thick, 24 px gap, `length_frac = 0.75`,
  centred) with a 0..1 placeholder z range.
- draw_style `Precise`; `polar`, `picked_points` and `picked_data` are `None`;
  no overlays, subplots or annotations.

Empty text is filled in via the `Chart::with_title / with_x_title / with_y_title / with_legend_entry` builders.

---

---

<a id="2-config-구조체-필드-레퍼런스"></a>

## Config 구조체의 필드

네이티브와 WASM은 같은 Rust 모델을 사용한다. JSON 형식은 [SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)에 있다.

극좌표(`Config::polar`, `LabelFormat::Degrees` / `Radians`), 레이더 차트(`RadarChart`), 벡터장(`DataRenderType::Vector`), 점 색·크기 매핑, 면 채움과 계단선(`fill`, `fill_color`, `interpolation`), `Config`의 `overlays`, `subplots`, `active_layer`, `annotations` 필드는 저장소 master 브랜치에 있으며 crates.io에 배포된 figgy-renderer 0.17.0 / figgy-model 0.11.0에는 아직 없다. 다음 릴리스 전에는 저장소를 의존성으로 지정한다.

```toml
renderer = { package = "figgy-renderer", git = "https://github.com/shim9610/figgy" }
```

무엇을 그릴지는 `Config`와 차트의 시리즈 목록이 정한다(SSOT). 세밀하게 조정하려면 현재 `Config`에서 원하는 필드만 직접 고친다. `min` / `max`만 바꾸면 설정한 눈금 간격과 라벨 형식은 그대로 남는다. 편의 헬퍼는 눈금이나 배치도 함께 정한다. 예를 들어 `Chart::set_x_range`는 두 X축의 눈금을 함께 조정하고, `Chart::with_title`은 제목 영역을 넓힐 수 있다. 작업별 표, 범위만 바꾸는 예제와 편집 뒤에 필요한 갱신 단계는 [설정 직접 편집과 편의 헬퍼(영문)](API.md#direct-ssot-edits-and-convenience-helpers)에 있다.

```rust
pub struct Config {
    pub chart_area: ChartArea,           // 패널 픽셀 영역(호스트 viewport 안)
    pub top_x: AxisOptions,              // 네 변의 축. 기본값은 top/right의 라벨·제목 숨김
    pub bottom_x: AxisOptions,
    pub left_y: AxisOptions,
    pub right_y: AxisOptions,
    pub chart_title: ChartTitleOptions,
    pub grid: GridOptions,
    pub legend: Legend,
    pub picked_points: Option<PickedPointsConfig>,
    pub picked_data: Option<DataSelectionsConfig>,
    pub colorbar: Option<ColorBarOptions>,   // 색상 막대이자 차트의 Z 스케일
    pub draw_style: DrawStyle,
    pub polar: Option<PolarOptions>,         // None이면 직교 차트. Some이면 bottom_x를 각도, left_y를 반지름으로 그린다
    pub overlays: Vec<OverlayLayer>,         // 오른쪽 보조 Y축. 각 축은 나열한 시리즈의 스케일을 맡는다
    pub subplots: Vec<Subplot>,              // 같은 문서에 그리는 추가 차트. 차트마다 축을 따로 가진다
    pub active_layer: LayerRef,              // 포인터 상호작용 대상 레이어. 기본값 `Base`는 차트 자체의 시리즈와 장식
    pub annotations: AnnotationLayer,        // 전용 주석 레이어: 표시 여부와 항목
}
```

### `ChartArea` / `Rect`
| 필드 | 타입 | 의미 |
|---|---|---|
| `x, y` | u32 | 호스트 화면의 왼쪽 위를 기준으로 한 패널 위치(px) |
| `width, height` | u32 | 패널 크기(px). 0이면 화면 렌더링은 `InvalidChartArea`로 실패한다. 출력 영역도 호출자가 양수로 지정해야 한다. 현재 출력의 1px 보정은 호환성을 위한 처리이며 향후 오류로 바뀔 수 있다. |

### `AxisOptions` (top_x / bottom_x / left_y / right_y)
| 필드 | 타입 | 의미 |
|---|---|---|
| `scale` | `AxisScale` | `Linear` 또는 `Logarithmic` (log10) |
| `min, max` | f64 | 데이터 좌표 범위. 로그축에서는 양수 경계를 그대로 쓴다. 로그축의 수동 최솟값이 0 이하이거나 유한하지 않으면 `1e-12`로, 최댓값이 최솟값 이하이거나 유한하지 않으면 최솟값의 10배로 보정한다. 로그축에 대응하는 선·산점도 좌표가 0 이하이면 해당 점을 그리지 않는다. 그 점에 닿는 선분도 생략해 빈 구간으로 남기며, 원본 데이터와 나머지 유효한 점은 유지한다. |
| `major_spacing` | f64 | 주 눈금 간격. 선형축은 데이터 단위, 로그축은 decade(10배 간격) 단위. 선형축 간격이 유한하지 않거나 0 이하이거나, 범위 끝값에 더하거나 빼도 값이 바뀌지 않을 만큼 작거나, 범위를 1000구간 넘게 나누면 자동 간격으로 그린다. 로그축 간격은 1 이상 2147483647(`i32::MAX`) 이하의 정수 중 그 값을 넘지 않는 가장 큰 수로 그리고, 1보다 작거나 유한하지 않으면 1로 그린다. 이렇게 바꾼 값은 다음 갱신 뒤 `ChartView::render_warnings`로 알린다. |
| `minor_count` | usize | 선형축에서는 주 눈금 사이의 보조 눈금 수. 로그축에서는 보조 눈금을 켜고 끄는 값으로만 쓴다. 0이면 숨기고, 1 이상이면 값과 관계없이 decade마다 2배~9배 위치 8곳에 그린다(`Chart::set_x_range` / `set_y_range`는 8을 쓴다). 선형축에서 99를 넘으면 주 눈금 사이에 99개만 그리고 같은 방식으로 알린다. |
| `inverted` | bool | 화면상의 축 방향을 뒤집는다. 눈금·격자·데이터·피킹이 같은 변환을 쓰며 `min` / `max`는 데이터 좌표 기준을 유지한다. |
| `label_style` | `LabelStyle` | 눈금 라벨 스타일 |
| `tick` | `TickVisibility` | `None / Outside / Inside / Both` |
| `title_option` | `AxisTitleOptions` | 축 제목의 텍스트·표시 여부·위치 |
| `out_margin` | f32 | 축 바깥의 라벨·제목 영역 여백(px) |
| `line_visible / line_color / line_width / line_style` | mixed | 축선 모양. `line_visible`은 축선만 제어하며 눈금(`tick`)과 라벨 표시 여부는 독립적이다. CPU 렌더링은 1px보다 가는 선도 사라지지 않도록 최소 두께를 1px로 적용한다. |
| `line_offset` | f32 | 데이터 영역을 유지한 채 축선·눈금·라벨을 축에 수직인 방향으로 이동한다. 전체 배치에는 영향을 주지 않으며 축 드래그 결과를 이 값에 기록한다. |
| `major_tick_length / minor_tick_length` | f32 | 눈금 길이(px) |

### `LabelStyle`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | 눈금 라벨을 그릴지 여부. `label_visible`과 둘 다 `true`일 때만 라벨을 그린다 |
| `color` | `Color` | 라벨 색 |
| `font_size` | f32 | px |
| `label_visible` | bool | 눈금 라벨 표시 여부. `visible`과 둘 다 `true`여야 라벨을 그리며, 어느 쪽을 꺼도 축선과 눈금은 그대로 그리고 `out_margin`도 바뀌지 않는다. |
| `label_font` | String | 글꼴 이름. 빈 문자열이면 내장 Liberation Sans를 쓴다. 등록되지도 설치되지도 않은 이름도 이 글꼴로 대체되며, 호스트는 `text_render::font_family_available(name)`으로 이름이 해석되는지 확인할 수 있다. 이 글꼴에 없는 글자는 글자마다 등록 글꼴, 내장 Liberation Sans, 설치된 시스템 글꼴(네이티브 전용) 순으로 찾아 그린다. |
| `label_offset_x / y` | f32 | 라벨 위치를 미세 조절하는 이동량(px). 극좌표 모드에서 각도 축은 `y`를 스포크 방향으로 읽고 `x`를 무시하며, 반지름 축은 `x`를 광선 방향, `y`를 수직 방향으로 읽는다(`PolarOptions` 참고) |
| `format` | `LabelFormat` | `Decimal / Power / Scientific / Timestamp / Degrees / Radians`. `Timestamp`는 선형축의 숫자 좌표를 Unix 시간으로 표시하며 달력 기준 눈금을 지원한다. `Degrees`는 소수에 도(°) 기호를 붙이고, `Radians`는 π의 배수(`π/2`, `3π/4`, `0.4π`)로 적는다. |
| `significant_digits` | u8 | 숫자 표시에 요청하는 정밀도(소수·과학 표기는 1~15). 선형 소수 라벨은 범위의 크기를 기준으로 소수 자릿수를 통일하고 눈금 간격을 구분하는 데 필요한 만큼만 늘린다. 로그 라벨은 값마다 반올림한다. 눈금 위치는 바뀌지 않는다. |

축을 따라 겹치는 숫자 라벨은 일정 간격으로 일부만 표시하며, 눈금과 격자 설정은 바꾸지 않는다. 정밀도를 바꿔도 `out_margin`이나 제목 위치를 자동으로 바꾸지는 않으므로 긴 라벨에는 여백을 직접 늘린다. 색상 막대의 눈금 라벨도 같은 형식 규칙을 사용한다.

`LabelFormat::Timestamp`는 숫자 좌표를 시간으로 표시한다. 기본값은 UTC 기준 Unix 초와 `AutoCalendar` 눈금 배치다. JavaScript 타임스탬프에는 `unit: TimestampUnit::Milliseconds`를, 한국 표준시(UTC+9)처럼 고정 시차를 적용하려면 `timezone: TimestampZone::FixedOffsetMinutes(540)`을 사용한다(타입은 `renderer::format`에 있다).
`AutoCalendar`는 라벨 너비를 측정해 달력상의 눈금 간격을 조절한다. 큰 Unix 시간 값에서도 작은 시간 차이를 보존하려면 네이티브에서는 `Renderer::add_hilo_column`, 웹에서는 `Float64Array`를 받는 `register_column_f64` / `update_register_column_f64`를 사용한다. GPU에는 `(hi: f32, lo: f32)` 쌍으로 저장된다.
[시간축 데모](https://github.com/shim9610/figgy/blob/master/crates/web/timestamp-demo.html)에서 시간 범위, 차트 너비와 출력 배율을 바꾸며 결과를 확인할 수 있다.

### `AxisTitleOptions` / `ChartTitleOptions`
| 필드 | 타입 | 의미 |
|---|---|---|
| `text` | `RichText` | 그리스 문자·위아래 첨자·굵게·기울임 등을 구간별로 지정한 텍스트. `align`(기본 `Left` / `Center` / `Right`)은 여러 줄 텍스트의 각 줄 위치를 정한다. |
| `visible` | bool | |
| `offset_x / y` | f32 | 제목 위치를 미세 조절하는 이동량(px). 여백 계산에는 반영하지 않으며 제목을 드래그한 결과가 여기에 누적된다. 좌·우 축 제목은 회전한 제목의 좌표계로 읽는다 |
| `top_margin` | f32 | 차트 제목 영역의 높이(`chart_title` 전용). 제목은 이 영역의 가운데에 놓인다. 제목 헬퍼(`Chart::with_title`, 웹 `set_title`)는 제목이 영역보다 높을 때 `layout::fit_chart_title_margin`으로 이 값을 늘리며, 설정에 직접 넣은 값은 그대로 둔다. |

### `GridOptions`
| 필드 | 타입 | 의미 |
|---|---|---|
| `show_major_x/y` | bool | 주 격자선 |
| `major_x/y_color, _width, _style` | mixed | 주 격자선 모양(Solid / Dash / Dot 등 프리셋 11종) |
| `show_minor_x/y` | bool | 보조 격자선 |
| `minor_x/y_color, _width, _style` | mixed | 보조 격자선 모양 |

### `DrawStyle`
| 종류 / JSON mode | 의미 |
|---|---|
| `Precise` / 생략 또는 `{ "mode": "precise" }` | 기본 정밀 렌더러. 기본 직렬화에서는 `draw_style` 키가 생략됨 |
| `Sketch` / `{ "mode": "sketch", ... }` | 차트 전체 손그림 스타일. 선·산점도·오차 막대 시리즈를 그리고 막대·필드·등고선·벡터는 그리지 않는다(`draw_style_supports`로 확인). 옵션 메타데이터는 `DrawStyle::param_specs_for_mode("sketch")`(웹: `draw_style_param_specs("sketch")`)에서 조회한다 |
| `Milkyway` / `{ "mode": "milkyway", ... }` | 차트 전체 천체사진 스타일. 선·산점도·오차 막대 시리즈를 그리고 막대·필드·등고선·벡터는 그리지 않는다(`draw_style_supports`로 확인). 옵션 메타데이터는 `DrawStyle::param_specs_for_mode("milkyway")`(웹: `draw_style_param_specs("milkyway")`)에서 조회한다 |
| `Constellation` / `{ "mode": "constellation", ... }` | `ScatterLine` 전용 별자리 스타일. 데이터 점 위치의 별과 반투명 연결선을 그린다. 별 크기는 `point_size`를 따른다. 옵션 범위는 `DrawStyle::param_specs_for_mode("constellation")`(웹: `draw_style_param_specs("constellation")`)에서 조회한다. |

### `PolarOptions` (`Config::polar`)

`polar: None`(기본값, JSON에서 키 생략)은 직교 차트다. `Some`이면 같은 시리즈를 극좌표로
그린다. `bottom_x`가 각도 축, `left_y`가 반지름 축이 되어 범위·스케일·눈금·라벨 스타일·제목을
그대로 쓰고, `top_x`와 `right_y`는 그리지 않는다. 데이터는 호스트의 (각도, 반지름) 컬럼 그대로다.
플롯은 데이터 영역 안에서 각도 라벨 한 줄이 들어갈 자리를 남긴 가장 큰 원이다
(`Config::polar_geometry`).

| 필드 | 의미 |
|---|---|
| `zero_angle_degrees: f64` | 각도 값 0이 가리키는 화면 방향(오른쪽에서 반시계 방향으로 잰 도). 0은 오른쪽(기본), 90은 위 |
| `clockwise: bool` | 각도 값이 화면에서 시계 방향으로 커지는지. 기본 `false`(반시계, 수학 관례) |
| `angle_unit: AngleUnit` | `Degrees`(기본) 또는 `Radians`. 각도 컬럼과 `sector`를 읽는 단위 |
| `sector: Option<[f64; 2]>` | `angle_unit` 단위의 가시 각도 창. 한 바퀴 이하이며 경계에서 클립된다. `None`(기본)은 전체 원이고, 축 범위 밖 각도는 `cos`/`sin`이 두는 자리에 그린다. 감싸기·정렬·중복 제거는 하지 않는다 |
| `inner_radius_fraction: f32` | 바깥 반지름에 대한 중앙 구멍 비율, `0.0..=0.9`. `left_y.min`이 이 고리에 놓인다 |
| `interpolation: PolarInterpolation` | `Chord`(기본): 두 표본 사이 세그먼트는 화면에서 직선. `Arc { pieces }`(1~64): (각도, 반지름)에서 선형 보간한 점들을 잇는 `pieces`개의 현으로, 반지름이 일정하면 고리를 따른다. 대시와 피킹도 같은 조각을 따라간다 |
| `radial_label_angle_degrees: f64` | 반지름 축 선과 눈금 라벨이 놓이는 광선의 화면 방향(도). 기본 90(위). 광선을 드래그하면 바뀐다 |

격자는 `*_y` 필드를 반지름 눈금의 고리로, `*_x` 필드를 각도 눈금의 스포크로 그린다. 각도 축은
바깥 고리이며 눈금은 바깥쪽을 향하고 라벨은 둘레에 놓인다. 전체 원에서는 두 눈금이 겹치는
자리(0과 360)에 스포크와 라벨을 하나만 그린다. 각도 라벨은 `LabelFormat::Degrees`(`90°`)와
`LabelFormat::Radians`(`π/2`, `3π/4`, `0.4π`)로 적는다. `left_y.min`보다 작은 반지름은
그리지 않으며(선은 거기서 끊긴다), 로그 반지름 축의 0 이하 표본은 직교 로그 축처럼
가린다. 그 밖에 시리즈가 그리는 모든 것은 플롯에 클립된다. 바깥 고리 밖(반지름 축을 확대해
`left_y.max`를 넘는 반지름), 구멍 안, 섹터 경계 너머의 도형은 직교 차트의 데이터 영역 scissor처럼 그
자리에서 잘리므로 표본이 섹터 밖에 있는 세그먼트도 경계까지는 그려진다. 섹터 밖 마커는 그리지도
고르지도 않으며, 선 조각은 그려진 곳에서 고를 수 있다. 선 채움(`LineFill::ToBaseline`)은 기준선 고리까지 채우며, 기준선이 축 시작값 이하이면 안쪽 고리(구멍이 없으면 중심)까지 채운다.

검증(`Config::validate_polar`, `InvalidConfig { field: "polar" }`로 보고): 유한한 각도,
범위 안의 `inner_radius_fraction`, `start < end`이고 한 바퀴 이하인 섹터, `1..=64`의 호 조각 수,
선형이며 뒤집지 않은 각도 축(`clockwise`를 쓴다), 그리고 `overlays` 없음. 극좌표 형태가 없는
시리즈는 `InvalidSeriesConfig`로 거부한다: 히스토그램, 히트맵, 등고선, 오차 막대, 벡터. `Arc`는
선형 보간 선이 필요하다. 스트리밍 소스는 거부한다. 주석은 비율 앵커만 쓰고 데이터 앵커는 건너뛴
것으로 보고한다. 서브플롯은 각자 `polar`를 가지므로 한 문서에 극좌표 패널과 직교 패널을 섞을 수
있다.

드래그는 극좌표 프레임을 따른다. 반지름 축 광선(`"axis_left"`)은 중심을 축으로 돈다. 포인터 위치가
있으면(`Draggable::drag_from`, `Config::nudge_from`, `on_move`가 보내는 값) 포인터의 각도를 그대로
따르고, 없으면(`drag_by`, 키보드 이동) 바깥 고리 위의 점이 드래그의 광선 수직 성분만큼 따라간다.
결과는 `radial_label_angle_degrees`에 기록된다. 각도 축은 바깥 고리, 곧 데이터 경계이므로
`"axis_bottom"`은 히트 영역도 분리(`line_offset`)도 없다. 각도 눈금 라벨(`"tick_labels_bottom"`)은
각자 자기 스포크를 따라 움직인다. 포인터 위치에서의 반경 성분(없으면 0도 스포크)이
`bottom_x.label_style.label_offset_y`에 누적되며 양수가 바깥쪽이고 `label_offset_x`는 효과가 없다.
반지름 눈금 라벨(`"tick_labels_left"`)은 광선 프레임에서 움직인다. `left_y.label_style.label_offset_x`는
광선 방향(바깥이 양수), `label_offset_y`는 수직 방향(라벨이 놓인 쪽, 광선에서 반시계 방향이 양수)이라
광선이 돌아도 같은 자리를 지킨다. 상단·우측 축은 그리지 않으며 제목을 포함해 아무것도 등록하지
않는다. 차트 제목, 하단·좌측 축 제목, 범례, 데이터 영역, 주석은 직교 차트의 규칙을 그대로 따른다.

### `LayerRef` / `AnnotationLayer`

모든 차트 종류(`Config`, `RadialChart`, `CategoricalChart`, `BoxPlotChart`,
`RadarChart`)가 공유하는 레이어 필드다.

| 필드 | 의미 |
|---|---|
| `active_layer: LayerRef` | 포인터 상호작용을 받는 레이어. `Base`(기본값, JSON에서 생략), `Overlay(index)`, `Annotations`, `Subplot(index)`, `SubplotOverlay(subplot, overlay)` 중 하나다. 히트 맵에 어느 레이어의 요소를 등록할지 정하며(`HitMap::for_active_layer`), `Annotations`일 때는 주석 항목을 차트 자체 히트 맵 위에서 먼저 검사하고 다른 레이어에서는 클릭이 주석을 통과한다(`ChartView::hit_test_active_layer`, `BoundedAnnotationFrame::hit_test_active_layer`). 요소 선택(`selected`, `picked_points`, `picked_data`)은 그 아래 단계이고 레이어가 바뀌면 데이터 선택을 뺀 요소 선택을 지운다. 범위 밖의 `Overlay`·`Subplot` 인덱스는 `Base`로, 오버레이가 없는 `SubplotOverlay`는 그 `Subplot`으로 읽고 저장한다. bounded 차트(원형·범주형 막대·박스플롯·레이더 차트)에는 오버레이와 서브플롯이 없으므로 이들은 `Base`로 되돌린다(`RadialChart`, `CategoricalChart`, `BoxPlotChart`, `RadarChart`의 `active_layer()` / `set_active_layer`, 바뀌면 세터가 `decorations.selected`를 지운다). |
| `annotations: AnnotationLayer { visible, items }` | 전용 주석 층. `items`를 그리는 순서대로 담으며, 구간(span)은 데이터 아래 층에, 나머지는 모든 장식 위 층에 합성하되 켜고 끄기·선택·편집은 한 단위로 다룬다. `visible = false`는 합성 두 번만 건너뛰고 어떤 래스터도 다시 만들지 않는다. 기본값(보임, 항목 없음)과 빈 `items`는 JSON에서 생략한다. |

오버레이 요소는 `DecorationId::Overlay { layer, inner }`(`overlay_<n>:<inner id>`, 아래 `OverlayLayer` 참고),
주석 항목은 `items` 안의 위치로 `DecorationId::Annotation(index)`(`annotation_<n>`)로 식별한다.

#### `Annotation`

| 필드 | 타입 | 의미 |
|---|---|---|
| `id` | `String` | 재정렬에도 유지되는 호스트용 자유 라벨. 기본값은 빈 문자열이며 그때 JSON에서 생략한다 |
| `visible` | `bool` | 숨긴 항목은 그리지도 히트 맵에 등록하지도 않고 기하만 보존한다. 기본 `true`, 그때 생략 |
| `kind` | `AnnotationKind` | 아래 네 종류 중 하나. 다른 enum처럼 외부 태그 방식(`"kind": {"Span": {...}}`)이다 |

| `AnnotationKind` | 필드 | 그리는 위치와 방식 |
|---|---|---|
| `ReferenceLine` | `axis: AnnotationAxis`, `value: f64`, `style: AnnotationLineStyle`, `label: Option<RichText>` | 장식 위 층. 플롯 영역 전체를 가로지르는 선. `X`면 세로선, `Y`면 가로선이며 라벨은 선의 위쪽(`X`) 또는 왼쪽(`Y`) 끝 안쪽에 둔다 |
| `Span` | `axis: AnnotationAxis`, `from: f64`, `to: f64`, `fill: Color` | 데이터 아래 층. 반투명 채움이 데이터를 물들이지 않는다. `from`과 `to` 사이의 띠를 플롯 영역에 잘라 그리며, 일부만 벗어나면 잘라 그리고 전부 벗어나면 건너뛴다 |
| `Text` | `anchor: Anchor`, `text: RichText`, `box_style: Option<BoxStyle>` | 장식 위 층. 텍스트 블록(상자가 있으면 상자)의 왼쪽 위 모서리를 앵커에 두며 패널 안이면 어디든 놓을 수 있다. 패널을 벗어나는 블록은 건너뛴다 |
| `Arrow` | `from: Anchor`, `to: Anchor`, `style: ArrowStyle`, `text: Option<RichText>` | 장식 위 층. `from`에서 `to`로 향하는 선과 `to` 끝의 채운 삼각형 머리. 라벨은 선 중점 옆 위쪽에 가운데 정렬한다 |

`AnnotationAxis`는 모든 차트에서 `X`(화면 가로)와 `Y`(화면 세로)다. Cartesian 차트는
데이터 변환이 쓰는 `bottom_x` / `left_y`에 대응하며 시리즈와 같은 선형·로그 규칙을 따른다
(로그축의 0 이하 값은 위치가 없다). 막대·박스 차트는 방향에 따라 대응한다. `Vertical`이면 `X`가
범주 인덱스, `Y`가 값 축이고 `Horizontal`이면 반대다. 범주 축은 범주를 세며 첫 범주의 중심이
좌표 `0`이므로 `-0.5..0.5`가 첫 범주의 슬롯이다. 원형·레이더 차트와 극좌표 `Config`(`polar: Some`)에는
분리된 값 축이 없으므로 `Ratio` 앵커만 놓을 수 있고, 데이터 앵커와 `ReferenceLine`·`Span`은 건너뛴 것으로
보고한다(`NoValueAxis`).

`Anchor`는 `{"Data": {"x": f64, "y": f64}}`(두 문서 축의 좌표. 확대·범위 변경을 따라간다)
또는 `{"Ratio": {"x": f32, "y": f32}}`(플롯 영역의 비율. `(0, 0)`이 왼쪽 위, `(1, 1)`이 오른쪽
아래이며 화면에 고정된다) 중 하나다. `0..1` 밖의 비율은 텍스트 상자나 화살표 끝을 플롯 밖,
패널 안에 둔다. `Anchor::converted(&space, kind)`는 화면 위치를 유지한 채 종류를 바꾼다.

스타일은 모두 `serde(default)`이며 기본값은 다음과 같다.

| 타입 | 필드 |
|---|---|
| `AnnotationLineStyle` | `color`(짙은 회색), `width: f32`(1.5), `style: LineStylePreset`(`Solid`) |
| `BoxStyle` | `fill`(흰색, 알파 0.9), `border_color`(짙은 회색), `border_width`(1.0), 테두리와 글자 사이 `padding`(4.0) |
| `ArrowStyle` | `line: AnnotationLineStyle`, `head_length`(10.0), `head_width`(8.0) |

선 굵기·글자 크기·여백·화살표 머리 같은 픽셀 치수는 `Config::scaled`로 차트의 다른 치수와
함께 배율이 적용되므로 2배 출력은 1배 히트 맵과 같은 기준으로 2배 크기로 그린다. 값과 앵커는
배율과 무관하다.

검증(`AnnotationLayer::validate`, 설정 검증에 포함)은 항목 1024개 이하, 제어 문자 없이 160자 이하인
`id`, `0..1`의 유한한 색, 선 굵기·상자 테두리 굵기·상자 여백 `0..64`, 화살표 머리 `0..256`, bounded
차트 제목과 같은 리치 텍스트 제한(글자 크기 6..160, 512 세그먼트, 8줄)을 본다. 유한하지 않은 값과 앵커는 오류가 **아니다**. 래스터가 그 항목을
건너뛰고 문서에는 그대로 두며 `RenderWarning::AnnotationSkipped { index, reason }`으로
알린다. 이유는 `NonFinite`, `NoValueAxis`(원형·레이더·극좌표 차트의 데이터 앵커, 기준선, 구간), `OutOfRange`(축
범위나 패널 밖), `EmptyText` 중 하나다. 주석은 데이터 영역 여백 계산에 참여하지 않으며(텍스트
상자를 추가해도 플롯이 줄지 않는다) 자동 라벨 배치기가 밀어내지도 않는다.

### `OverlayLayer` (`Config::overlays`)

오버레이 레이어는 공유 데이터 영역의 오른쪽에 그리는 두 번째 Y 축이며, 자기가 나열한 시리즈를
그 축으로 스케일한다. `overlays`의 기본값은 빈 목록이며 그때 JSON에서 키를 생략한다.
레이어가 있어도 문서는 하나의 `Config`다. 차트 영역, 두 X 축, 제목, 그리드, 합친 범례,
색상 막대, `draw_style`은 부모에 한 번만 있고 아래 필드만 레이어마다 있다. bounded 차트
(원형·범주형 막대·박스플롯·레이더)에는 오버레이가 없고, 비상주 스트리밍은 오버레이가 있는 문서를 거부한다.

| 필드 | 타입 | 의미 |
|---|---|---|
| `y_axis` | `AxisOptions` | 오른쪽에 그리는 오버레이 Y 축. `min` / `max` / `scale` / `inverted`가 이 레이어 시리즈의 픽셀 변환을 정한다. `out_margin + major_tick_length`가 공유 오른쪽 여백 안의 띠이며, 오른쪽 여백은 부모 `right_y`와 모든 오버레이 띠 중 가장 넓은 값이다(`Config::margins`). 기본값은 왼쪽 축 기본 옵션(라벨·축선 보임)이다. |
| `series` | `Vec<String>` | 이 레이어가 스케일하는 시리즈 id. 차트의 단일 시리즈 목록에서 참조한다. 한 id는 최대 한 오버레이에만 속할 수 있고(`Config::validate_overlays`), 선언이 없는 id는 무시하며, 나머지 시리즈는 기본 레이어에 속한다. 비어 있으면 JSON에서 생략한다. |
| `legend` | `Option<Legend>` | 이 레이어 패널에 그리는 범례. `None`(생략)이면 부모의 합친 범례 하나가 모든 레이어의 시리즈를 나열한다. |

렌더러는 `Config::expand()`로 문서를 레이어마다 하나의 내부 패널로 그린다. 기본 패널은
오버레이를 뺀 부모이고, 오버레이 패널은 변환을 위해 부모의 X 범위를 유지하되 X 축·그리드·
제목·색상 막대를 그리지 않으며, 숨긴 왼쪽 축에 오버레이 범위를 싣고 오른쪽에 오버레이
`y_axis`를 그린다. 모든 패널의 여백을 그룹 여백으로 올리므로 `data_area()`는 모든 패널과
부모에서 같은 사각형이며 `Expansion { panels, data_area }`로 돌려준다. 확장은 문서를
바꾸지 않고, 확장한 패널은 직렬화하지 않으므로 JSON에는 부모 `Config` 하나만 저장된다. 데이터 영역 가장자리 드래그나 `fit_to_data_area`는
부모 `right_y.out_margin`으로 그룹 띠를 쓰고 모든 오버레이 띠를 그 안으로 맞춘다.
`Config::scaled`는 오버레이 축과 범례도 부모와 같은 규칙으로 배율을 적용한다. 부모의
`right_y`는 적힌 대로 두므로, 오버레이 축이 오른쪽 띠를 독점하게 하려면 그 축선과 눈금을 끈다.

`active_layer`는 상호작용 대상 레이어 하나를 가리키고(`Config::active_layer()`는 범위 밖
인덱스를 `Base`로 읽고 `set_active_layer`가 저장한다) `Config::layer_of_series(id)`는 시리즈를
스케일하는 레이어를 답한다. 히트 맵·선택·출력 동작은 [상호작용 가이드(영문)](INTERACTION.md#overlay-layers)에 있다.
`Chart::set_overlay_y_range(layer, min, max)`는 오버레이의 범위를 정하고 눈금을 함께 맞추며,
[examples/overlay_axes.rs](examples/overlay_axes.rs)는 두 Y 축을 그리는 예제다.

### `Subplot` (`Config::subplots`)

서브플롯은 같은 문서에 그리는 차트 하나다. `subplots`의 기본값은 빈 목록이며 그때 JSON에서
키를 생략한다. 각 서브플롯은 자기 `chart_area`(부모와 같은 표면 좌표), 축 4개,
제목, 그리드, 범례, 색상 막대, 스타일, 보조 Y 축을 모두 가진 완전한 Cartesian 차트이며,
링크로 묶지 않으면 부모나 다른 서브플롯과 아무것도 공유하지 않는다. 문서 자체의 `Config`가
첫 번째 차트이므로 2×2 그리드는 부모 하나와 서브플롯 셋이다.

| 필드 | 타입 | 의미 |
|---|---|---|
| `config` | `Config` | 서브플롯의 차트. 그 `overlays`는 서브플롯 안의 보조 Y 축이다(중첩은 한 단계라서 그 `subplots`는 비어 있어야 한다). 주석 층은 문서에 하나뿐이고 부모에 있으므로 그 `annotations`는 기본값이어야 하며, 그 `active_layer`는 무시한다. |
| `series` | `Vec<String>` | 이 서브플롯이 그리는 시리즈 id. 문서의 단일 시리즈 목록에서 참조하며 서브플롯 오버레이가 스케일하는 시리즈도 포함한다. 한 id는 최대 한 서브플롯에만 속하고 부모 오버레이에는 속할 수 없다(`Config::validate_subplots`). 선언이 없는 id는 무시한다. 비어 있으면 JSON에서 생략한다. |
| `link_x` | `Option<CellRef>` | `None`(기본값, 생략)이면 X 축은 서브플롯 자신의 것이다. `Some(CellRef::Parent)` 또는 이 서브플롯보다 작은 `j`의 `Some(CellRef::Subplot(j))`이면 `bottom_x`의 범위·스케일·방향·눈금 간격(`major_spacing`, `minor_count`)이 대상의 `bottom_x`를 따른다. 축선·라벨·제목 스타일은 서브플롯 자신의 것이다. |
| `link_y` | `Option<CellRef>` | `left_y`에 대해 같은 규칙. |

링크 묶음에는 값이 하나뿐이다. `Config::subplot_config(i)`는 링크된 축을 체인의 뿌리에서 복사해
서브플롯 `i`를 해석하고, `Config::sync_subplot_links(previous)`는 링크된 필드를 뿌리와 같게
저장한다. 편집 전 문서(`previous`)를 넘기면 구성원 축의 편집을 먼저 뿌리로 옮기므로, 링크된 축에
새 범위를 쓰면 묶음 전체가 움직인다. 같은 호출에서 뿌리와 구성원을 함께 고쳤다면 뿌리의 편집이
이긴다. `Renderer`는 차트를 등록할 때와 설정·상태·뷰 상태를 바꿀 때마다(`register_chart`,
`set_chart_config`, `set_chart_state`, `set_chart_view_state`) 이를 실행하므로, 링크 편집은
`&mut self` 호출 한 번 안에서 모든 차트에 반영된다.
`Chart::set_subplot_x_range` / `set_subplot_y_range`와 `auto_fit_subplot_{x,y}_{extent,union}`은
링크된 축의 뿌리에 직접 쓴다.

렌더러는 모든 서브플롯을 부모와 같은 규칙으로 전개하며(`Config::expand`. 서브플롯 패널은 부모 패널
뒤에 오며 순서는 `Config::panel_layers`가 알려 준다) 각 서브플롯은 자기 영역과 자기 데이터 영역을 갖는다. 문서는 모든 차트
영역의 합집합을 덮는다(`Config::document_area`). 문서 출력은 이 사각형을 그리고, 화면 표시는
이 사각형을 표면에 맞추면서 모든 차트를 부모와 함께 옮긴다(`Config::translate_areas`,
`display_config_for_surface`, `place_document`). `Config::scaled`는 서브플롯에도 부모와 같은
규칙으로 배율을 적용한다. 선택과 드래그는 [상호작용 가이드(영문)](INTERACTION.md#subplots)에 있고,
[examples/subplots.rs](examples/subplots.rs)는 X 축을 링크한 2×2 그리드 예제다.

### `Legend`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | |
| `content` | `RichText` | 범례 전체를 하나의 리치 텍스트로 저장한다. `\n`은 줄바꿈이며 기호는 구간별 색을 지정한 텍스트다. 줄바꿈·기호 위치·문자 사이의 기호까지 설정에 명시한다. `font` / `font_size`는 그리기 시 적용한다. |
| `corner` | `LegendCorner` | `TopLeft / TopRight / BottomLeft / BottomRight` |
| `padding` | f32 | 범례 상자 내부 여백. 모서리 배치는 데이터 영역 안쪽의 고정 간격에 `offset_x / offset_y`를 더한다. |
| `offset_x, offset_y` | f32 | 모서리 기준 위치에서의 이동량(화면 px). 여백 계산에는 반영하지 않으며 범례를 드래그한 결과가 여기에 누적된다. |
| `bg_color, border_color` | `Color` | 박스 배경 / 테두리 |

범례 기호는 `field_em`으로 너비를 고정한 텍스트 구간이다. 모양에 관계없이 `SYMBOL_FIELD_EM`, 즉 폰트 크기의 2배 너비를 차지한다. 선 기호는 `rule: true`로 구간 전체에 수평선을 그리고, 점 기호는 가운데에 `● ■ ▲ …` 같은 글리프를 놓는다. 선과 점을 함께 표시할 때도 전체 너비는 같다.
점선·도트 패턴은 `rule_dash`에 저장하므로 범례에도 `LineStylePreset`의 선 모양이 반영된다. 자동으로 만든 항목은 `심볼 + ' ' + '\t' + 라벨` 형식이며, 탭을 기준으로 라벨이 정렬된다. 직접 구성할 때는 `symbol_segments(kind, color)`, `series_symbol_segments(cfg)`, `append_legend_entry(content, symbol, label)`을 사용한다.

### `PickedPointsConfig`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | `picked_points`가 있을 때 선택 표시를 그릴지 여부 |
| `refs` | `Vec<PickedPointRef>` | 선택한 데이터의 `series_id`, 선택적인 `source_id`, `point_index`. 좌표 복사본 대신 출처와 인덱스를 저장한다. |
| `ring_color` | `Color` | 강조 테두리 색 |
| `ring_width_px` | f32 | 강조 테두리의 두께(px) |
| `radius_extra_px` | f32 | 점 기호 바깥으로 추가할 반지름 |

`picked_points`가 없거나 JSON `null`이면 점 선택 표시를 그리지 않는다. `{}`는 기본 설정인 `visible: true`, 빈 참조 목록, 금색 테두리, 선 두께 2px, 추가 반지름 3px로 해석한다. 이 설정에 `refs`를 채워 선택한 점을 표시할 수 있다.
강조 테두리의 크기는 점별 스타일을 포함한 산점도 기호의 반지름을 따른다. 선만 있는 시리즈에서는 선택된 끝점 주위에 `radius_extra_px`만큼의 반지름으로 그린다.

### `DataSelectionsConfig`

`Config.picked_data`는 `PickedDataRef`로 선택 대상을 저장한다. `Point`, `HistogramBin`, `MatrixCell`, `ContourLevel`로 종류를 구분하며, 모두 `series_id`와 선택적인 `source_id`를 가진다. 종류에 따라 `point_index`, `bin_index`, X·Y축 기준 `x_index/y_index`, 또는 `level_index + x_index/y_index`가 추가된다.
표시 모양은 `highlight_color`, `outline_width_px`, `point_radius_extra_px`, `contour_width_extra_px`로 설정한다. 좌표·막대 경계·등고선 선분은 복사해 보관하지 않고, 일반 그리기와 같은 GPU 자원에서 계산한다. 범위를 벗어난 인덱스는 그리지 않는다. JSON `null`은 선택 표시를 해제하고, `{}`는 선택 대상이 없는 금색 기본 설정을 만든다.

### `ColorBarOptions`
| 필드 | 타입 | 의미 |
|---|---|---|
| `visible` | bool | `false`이면 색상 막대와 그 여백을 없앤다. Z 범위와 색상표는 계속 적용되므로 행렬 시리즈와 색 매핑을 쓰는 점·벡터는 그대로 그린다. |
| `side` | `Side` | `Left`/`Right` = 수직 바, `Top`/`Bottom` = 수평 바. 이것만으로 방향이 결정된다 |
| `thickness_px` | f32 | 색상 막대 두께 |
| `gap_px` | f32 | 색상 막대와 그 안쪽(데이터 영역 쪽) 차트 축 영역 사이의 간격(px) |
| `length_frac` | f32 | 데이터 영역에서 막대를 놓은 변의 길이에 대한 막대 길이 비율. `(0, 1]` 범위의 값을 쓴다 |
| `align` | `BarAlign` | 변을 따라 시작·가운데·끝에 배치하는 `Start` / `Center` / `End` |
| `offset_x`, `offset_y` | f32 | 기준 위치에서의 이동량(px). 드래그 결과가 누적된다. 여백 계산에는 반영하지 않으므로 막대를 움직여도 데이터 영역은 바뀌지 않는다. |
| `colormap` | `ColorMap` | `Viridis` / `Magma` / `Turbo` / `GrayScale` / `RdBu` / `Custom { stops }` |
| `nan_color` | `Color` | 색으로 변환할 수 없는 Z값(NaN, 로그 스케일의 0 이하 값)에 사용할 색. 기본값은 완전 투명이다. |
| `border_color`, `border_width` | `Color`, f32 | 색상 막대 테두리 |
| `axis` | `AxisOptions` | Z축 범위와 눈금·라벨 설정 |

색상 막대의 `axis`는 차트의 네 축과 같은 `AxisOptions`를 사용한다. `scale`과 `Logarithmic`, `min`/`max`, `major_spacing`, `minor_count`, `label_style`과 `LabelFormat::Power`, `tick`, `title_option`의 의미가 모두 같다. 눈금 생성, 라벨 형식, 로그 처리를 같은 코드로 수행한다.

배치와 동작 규칙은 다음과 같다.

- `Heatmap` / `Contour` / `HeatmapContour` 시리즈와 `point_color_map` 또는 벡터 `color_map`을 쓰는 시리즈에는 색상 막대 설정이 반드시 필요하다. 이 설정에서 Z축 범위와 색상표를 읽으므로, 설정이 없으면 렌더러가 시리즈를 거부한다.
- 차트 하나는 Z축 스케일 하나를 사용한다. 여러 히트맵을 넣어도 같은 스케일을 공유한다.
- 색상 막대가 차지하는 여백은 `gap_px + thickness_px + axis.out_margin + axis.major_tick_length`이며 배치한 쪽에만 추가된다. `fit_to_data_area` / `resize_chart_area_scaled`는 라벨 공간인 `axis.out_margin`만 조절한다. 막대 자체의 두께는 창 크기에 따라 줄어들지 않는다.
- 일반 축과 다른 기본값은 `line_visible: false`와 `tick: Outside`다. 축선은 막대 테두리가 대신하고, 눈금은 색상 영역 바깥의 라벨 공간에 놓인다.
- 색상 막대는 해당 방향의 여백 중 가장 바깥쪽에 배치한다. 차트 가장자리에서 안쪽으로 라벨 공간, 눈금, 색상 막대, `gap_px`, 차트 축 영역 순서다. 이렇게 배치해야 데이터 영역 바깥으로 그려지는 축 라벨을 색상 막대가 가리지 않는다.
- 색상 막대는 별도 GPU 파이프라인 없이 CPU 장식 레이어에서 그린다. 일반 축과 같은 코드로 로그 눈금과 10ⁿ 라벨을 만든다. `axis.tick`은 눈금의 안쪽·바깥쪽 표시를, `axis.inverted`는 화면상의 값 증가 방향을 정한다. 눈금 모양은 축선의 `line_color` / `line_width` / `line_style`을 따른다.
- 색상 막대도 선택·드래그·크기 조절을 지원한다. 히트테스트 ID는 `"colorbar"`이며, 데이터 영역과 마찬가지로 파란 선택 상자와 조절점 8개를 표시한다. 드래그는 `offset_{x,y}`에 누적한다. 크기 조절은 막대 방향에 따라 `thickness_px` 또는 `length_frac`을 바꾼다. 세로 막대에서는 좌우 조절점이 `thickness_px`를, 위아래 조절점이 `length_frac`을 바꾸며 가로 막대에서는 반대다.
- 세부 요소인 `"colorbar_axis"`, `"colorbar_tick_labels"`, `"colorbar_title"`도 각각 선택할 수 있다. 드래그하면 축선·라벨·제목의 오프셋이 바뀐다. 모두 실제 색상 막대 사각형을 기준으로 계산하므로 길이·정렬·위치·크기가 바뀌어도 제목과 선택 영역이 막대를 따라간다.
- `ColorBarOptions::normalized_z(z) -> Option<f32>`가 Z값을 색상 위치로 변환하고, `color_for_z`가 그 결과를 적용한다. 유효한 결과는 `[0,1]`로 제한한다. NaN, 로그 스케일의 0 이하 값, 너비가 없는 범위는 `None`을 반환해 `nan_color`로 표시한다. `axis.inverted`는 화면상의 위치만 바꾸며 값에 대응하는 색은 바꾸지 않는다.

<a id="data_config--series-선언형-스키마-활성-api"></a>
<a id="data_config--시리즈-선언-형식"></a>

### `data_config` 시리즈 선언 형식

차트의 시리즈는 `data_config::SeriesConfig`로 선언한다. `Renderer::paint`는 `render_type`에 따라 선·점·오차 막대 등의 레이어를 만들고 각 스타일 설정에서 색·두께·모양을 읽는다.

| 타입 | 필드 | 역할 |
|---|---|---|
| `SeriesConfig` | `series_id, source_id?, label, x_column: ColumnId, y_column: ColumnId, render_type` | 시리즈 설정. `source_id`는 생략할 수 있으며, 피킹·선택 결과에서 호스트가 데이터 출처를 구분하는 데 쓴다. 컬럼 ID는 상주 풀 또는 재공급 가능한 비상주 소스를 가리킨다. 웹 편집 중 표시할 라벨은 `legend.content`를 기준으로 하며 일반 시리즈 편집은 기호만 갱신한다. `SeriesConfig.label`은 `reset_legend_from_series_labels()`로 범례를 다시 만들 때 사용한다. |
| `DataRenderType` | 14종 열거형 | 종류마다 별도의 렌더링 경로와 필요한 설정을 가진다. |
| `ErrorRef` | `Symmetric { column }` 또는 `Asymmetric { lower, upper }` | 오차 막대 컬럼 참조. Symmetric은 ±σ, Asymmetric은 아래·위 값을 별도 지정 |
| `DataLineStyleConfig` | `line_style, line_color, line_width, fill?, fill_color?, interpolation` | 선 모양. `fill`은 선 아래(`ToBaseline(y)`) 또는 다른 시리즈까지(`ToSeries(series_id)`) 면을 채우고, `fill_color`는 생략 시 `line_color`의 알파를 1/4로 줄인 색이며, `interpolation`은 `Linear`(기본)·`StepPre`·`StepPost`·`StepMid`다. [면 채움과 계단선](#면-채움과-계단선) 참고 |
| `DataScatterStyleConfig` | `point_color, point_shape, point_size, point_style_table?, point_style_index_column?, point_style_overrides?, point_color_map?, point_size_map?` | 점 모양. 정밀 모드에서 선택적인 스타일 표·개별 설정으로 색·모양·크기의 전체 또는 일부를 바꿀 수 있다. `point_color_map`은 값 컬럼을 `Config.colorbar`로 색에, `point_size_map`은 값 컬럼을 명시한 범위로 크기에 대응시킨다. [점 색과 크기 매핑](#점-색과-크기-매핑) 참고 |
| `DataErrorBarStyleConfig` | `error_bar_color, _width, _cap_size, cap_width, error_bar_style_table?, error_bar_style_index_column?, error_bar_style_overrides?` | 오차 막대 모양. 정밀 모드에서는 선택적인 스타일 표·개별 설정으로 색·몸통 두께·끝선 절반 길이·끝선 두께의 전체 또는 일부를 바꿀 수 있다. |
| `DataBarStyleConfig` | `fill_color, border_color, border_width, baseline, gap_px, width_ratio, orientation, bar_style_overrides?` | 히스토그램 모양. `width_ratio`는 구간 가운데에 놓인 막대의 너비 비율이다. 개별 설정으로 특정 구간의 채움·외곽선·간격·너비를 바꿀 수 있다. |
| `ScatterShape` | 열거형 26종 | 원·사각형·방향별 삼각형·마름모·십자·더하기·오각형·육각형·팔각형·별과 내부를 채운 형태 |

**`DataRenderType`의 14가지 종류**

| 종류 | 사용하는 스타일 | 의미 |
|---|---|---|
| `Line { line }` | line | 선만 |
| `Scatter { scatter }` | scatter | 점만 |
| `ScatterLine { scatter, line }` | 둘 다 | 점 + 연결선 |
| `ScatterErrorbarX { scatter, err_x, err_style }` | scatter + errorbar | 점 + X 오차 막대 |
| `ScatterErrorbarY { scatter, err_y, err_style }` | scatter + errorbar | 점 + Y 오차 막대 |
| `ScatterErrorbarXY { scatter, err_x, err_y, err_style }` | scatter + errorbar | 점 + X/Y 오차 막대 |
| `LineScatterErrorbarX / Y / XY` | line + scatter + errorbar | 위 3 + 연결선 |
| `Histogram { bar }` | bar | 호스트가 집계한 경계·빈도 `(edges, counts)`로 막대를 그린다. `bar.orientation`이 `Vertical`이면 X컬럼이 경계, Y컬럼이 빈도이며 `Horizontal`은 반대다. `edges = counts + 1`이라는 길이 관계로 역할을 추측하지 않는다. |
| `Heatmap { matrix, fill }` | fill | 면만 |
| `Contour { matrix, contour }` | contour | 선만 |
| `HeatmapContour { matrix, fill, contour }` | fill + contour | 면 + 그 위의 선 |
| `Vector { components, style }` | vector | 표본마다 화살표 하나. `components`는 X·Y 앵커와 정렬된 (u, v) 컬럼 쌍(`Points`) 또는 좌표 컬럼 위의 행렬 두 개(`Grid`)이고, `style`은 색·크기·피벗·스케일·선택적 색상 매핑이다. [벡터장](#벡터장) 참고 |

세로·가로 설정과 기존 차트의 방향 변경 예제는 [히스토그램 사용법](HISTOGRAM.md)에 있다.

<a id="점-색과-크기-매핑"></a>

#### 점 색과 크기 매핑

마커를 그리는 모든 `DataRenderType`은 일반 값 컬럼 두 개를 점에 대응시킬 수 있다. 두 필드는 모두 선택 사항이며 생략하면 JSON에서 빠지므로, 두 필드가 없는 문서는 읽고 다시 써도 바이트 단위로 같다. 둘 중 하나라도 지정한 시리즈는 스타일 표를 쓴 시리즈처럼 정밀 매핑 산점도 경로로 그린다.

- `point_color_map: Option<PointColorMap { column }>`은 `column`의 값으로 점마다 색을 정한다. 히트맵 셀과 똑같이 `Config.colorbar`를 거치므로 범위·스케일·색상표·`nan_color`는 색상 막대가 정하고 차트의 Z 스케일은 하나로 유지된다. 색상 막대가 없으면 임의의 범위를 쓰지 않고 시리즈를 거부한다. 유한하지 않은 값과 로그 색상 막대의 0 이하 값은 `nan_color`로 그린다. `Chart::auto_fit_colorbar_union(pool, ids, padding)`은 넘겨받은 컬럼 ID로 공유 범위를 맞추며 행렬 컬럼과 색 매핑 컬럼을 구분하지 않는다. 웹 `auto_fit_colorbar`는 차트의 행렬 컬럼, 점 색 매핑 컬럼, 벡터 색 매핑 컬럼을 모두 모아 맞춘다.
- `point_size_map: Option<PointSizeMap { column, domain_min, domain_max, min_px, max_px, scale }>`은 `domain_min..domain_max`를 마커 반지름 `min_px..max_px`에 대응시킨다. 단위는 `point_size`와 같은 논리 픽셀이며 출력 배율도 똑같이 적용된다. 범위는 유한하고 `domain_min < domain_max`여야 하며, `min_px`와 `max_px`는 유한한 0 이상의 값이고 `min_px <= max_px`여야 한다. `scale`은 `SizeScale::Area`(기본값. 값이 마커 넓이에 비례하며 버블 크기를 읽는 일반적인 방식) 또는 `Radius`다. 범위 밖 값은 양 끝으로 고정하고, 유한하지 않은 값은 점을 숨기며 피킹에서도 뺀다. 범위는 컬럼에서 추측하지 않는다. `PointSizeMap::fit_domain(min, max)`는 `auto_ticks`가 눈금 간격을 쓰듯 호스트가 이미 가진 통계로 범위만 쓰므로, 점을 추가해도 다른 점의 크기가 바뀌지 않는다. 반지름 공식은 `PointSizeMap::radius_px(value)`이며 셰이더와 CPU 피커가 같은 값을 계산한다.
- 채널별 우선순위는 개별 설정 > 값 매핑 > 스타일 표 슬롯 > 시리즈 기본값이다. 모양은 계속 표·개별 설정·기본값에서 정하므로 범주형 모양 컬럼과 연속 색 컬럼을 함께 쓸 수 있다. `point_size`를 지정한 개별 설정은 숨겨진 점을 다시 보이게 한다.
- 값 컬럼은 시리즈의 x/y 컬럼과 인덱스가 맞아야 하고 모든 점을 덮어야 하며, 더 짧으면 준비 단계에서 오류가 난다. 다른 시리즈 컬럼과 똑같이 다루므로 컬럼을 지우면 시리즈가 무효가 되고, 피킹(`Renderer::pick_chart`, 웹 `pick_point`), 선택 테두리, 웹 `selection_details`(`color_value` / `size_value`), PNG 출력이 모두 매핑된 색과 반지름을 따른다. 매핑에는 GPU 상주 컬럼이 필요하므로 스트리밍 소스는 거부한다. Sketch·Milkyway·Constellation 스타일은 스타일 표와 마찬가지로 매핑을 무시한다.

GPU는 그릴 때마다 컬럼 풀에서 값 컬럼을 읽어 셰이더에서 색과 반지름을 계산하므로, 값 컬럼을 갱신하면 다음 그리기부터 점에 반영된다. [examples/value_maps.rs](examples/value_maps.rs)에 두 매핑을 함께 쓰는 예제가 있다.

<a id="벡터장"></a>

#### 벡터장

`DataRenderType::Vector { components, style }`는 표본마다 화살표 하나를 그린다. `components`는 (u, v) 성분을 읽을 곳이며, 다른 종류와 마찬가지로 시리즈의 `x_column` / `y_column`이 기준점이다.

- `VectorComponents::Points { u, v }`: 기준점과 인덱스가 맞는 값 컬럼 두 개. 인덱스마다 화살표를 하나 그리며, 개수는 네 컬럼(`VectorColorMap::Column`이 있으면 다섯 컬럼) 중 가장 짧은 길이다.
- `VectorComponents::Grid { u, v }`: 히트맵 행렬과 똑같이 시리즈의 좌표 컬럼 위에 선언한 `MatrixRef` 두 개. 두 격자의 `orientation`과 `grid_layout`이 같아야 하며 다르면 거부한다. `Centers`에서는 좌표 위에, `Edges`에서는 셀 중심(두 경계의 데이터 좌표 중점이라 로그축에서는 화면 중점과 다르다)에 화살표를 놓는다. 두 격자가 공통으로 덮는 범위까지만 그리고, 짧은 히트맵 행렬처럼 잘림으로 보고한다. 같은 격자의 히트맵·등고선과 좌표 컬럼을 함께 쓸 수 있으며, 촘촘한 필드 위에 성긴 화살표를 그리려면 더 성긴 좌표 컬럼을 따로 등록한다.

`VectorStyle`은 화살표 모양이다.

- `color`는 `color_map`이 없을 때의 화살표 색이다. `width_px`(몸통 두께), `head_length_px`, `head_width_px`는 `point_size`와 같은 논리 픽셀이며 출력 배율이 적용되고, 유한한 0 이상의 값이어야 한다. 화살표가 머리보다 짧으면 머리를 화살표 길이에 맞춰 줄인다.
- `scale`: `VectorScale::Pixels { px_per_unit }`는 `px_per_unit * |(u, v)|` 픽셀 길이로 그리며, 방향은 두 축의 단위당 픽셀 비율을 따른다. 그래서 (1, 1) 벡터는 데이터 영역의 가로세로 비율만큼 기운다. `VectorScale::Data { factor }`는 축 변환을 거쳐 끝점을 데이터 좌표 `anchor + factor * (u, v)`에 두므로, 화살표가 뒤집힌 축이나 로그축을 따르고 끝점도 자동 범위 맞춤에 들어간다. 배율은 유한하고 0이 아니어야 한다. `VectorStyle::fit_scale(u_bounds, v_bounds, target)`은 호스트가 이미 가진 성분 통계로, 그 범위에서 가능한 가장 긴 벡터가 `target`(`Pixels`는 논리 픽셀, `Data`는 데이터 단위)이 되도록 배율만 쓴다. 따라서 표본을 추가해도 다른 화살표의 길이가 바뀌지 않는다.
- `pivot`: `Tail`(기본값, 기준점이 꼬리), `Middle`, `Tip`.
- `min_length_px`: 배율을 적용한 길이가 이보다 짧은 화살표는 그리지 않는다. 0 벡터와 유한하지 않은 성분이나 기준점도 그리지 않는다.
- `color_map: Option<VectorColorMap>`: 점 형식에는 `Column(id)`, 격자 형식에는 `Grid(MatrixRef)`를 쓰며 성분 형식과 맞아야 한다. 값은 히트맵 셀이나 `point_color_map` 마커와 똑같이 `Config.colorbar`로 색이 정해지며, 색상 막대가 없으면 시리즈를 거부한다.

상호작용은 성분 형식을 따른다. 점 벡터는 마커처럼 기준점에서 피킹하며, 반지름은 머리와 몸통 중 넓은 쪽의 절반이다. 선택 테두리, 웹 `step_selection`과 `selection_details`(`u`, `v`, `color_value`), PNG 출력도 같은 기준을 쓴다. 격자 벡터는 히트맵 셀처럼 `u` 격자의 셀 단위로 피킹하고 강조한다. 자동 범위 맞춤(`begin_series_fit_extent`, 웹 범위 요청)은 기준점과 데이터 배율 점 벡터의 끝점을 포함하며, 격자 벡터의 데이터 배율 끝점은 포함하지 않는다. 성분에는 GPU 상주 컬럼이 필요하므로 스트리밍 소스는 거부한다. Sketch·Milkyway·Constellation 스타일은 벡터를 그리지 않는다(`draw_style_supports`가 `false`를 돌려준다). 범례 기호는 시리즈 색의 화살표 머리가 달린 선이다.

셰이더는 직교 투영, 입력 형식, 화살표 글리프, 진입점 조각을 조립해 만들며([src/data_render/SHADER_COMMON.md](src/data_render/SHADER_COMMON.md) §5.3), 그릴 때마다 컬럼 풀에서 성분 컬럼을 읽는다. [examples/vector_field.rs](examples/vector_field.rs)에 두 형식의 예제가 있다.

<a id="면-채움과-계단선"></a>

#### 면 채움과 계단선

선을 그리는 모든 종류(`Line`, `ScatterLine`, `LineScatterErrorbar*`)는 세 가지 선택 설정을 읽는다. 모두 기본값이면 JSON에서 생략하므로 기존 문서는 그대로 읽고 쓴다.

- `fill: Option<LineFill>`은 선과 기준 사이의 면을 채운다. `LineFill::ToBaseline(y)`는 데이터 값 `y`까지(보통 `0.0`), `LineFill::ToSeries(series_id)`는 지정한 시리즈의 선까지 같은 인덱스끼리 채운다(오차 막대가 추가 컬럼을 읽는 방식과 같다). 두 시리즈 사이 채움은 **같은 x 컬럼**을 써야 하며, x 컬럼이 다르거나 시리즈가 없거나 자기 자신을 가리키면 재표본화하지 않고 `InvalidSeriesConfig` 오류를 돌려준다. 상대 시리즈의 `interpolation`은 읽지 않고 이 선의 보간을 따른다.
- `fill_color: Option<Color>`는 채움색이다. 생략하면 `line_color`의 알파에 `DataLineStyleConfig::DEFAULT_FILL_ALPHA`(0.25)를 곱한 색을 쓰며, `effective_fill_color()`가 실제로 칠하는 색을 돌려준다.
- `interpolation: LineInterpolation`은 표본 사이를 직선(`Linear`) 또는 축에 평행한 계단 조각으로 잇는다. `StepPre`는 `x[i]`에서 `y[i + 1]`로 먼저 오른 뒤 `x[i + 1]`까지 수평으로, `StepPost`는 `y[i]`로 `x[i + 1]`까지 간 뒤 그 자리에서 오르고, `StepMid`는 두 x 값의 **데이터 좌표 중점**에서(픽셀 중점이 아니다. 로그축에서는 둘이 다르다) 오른다. 점선 위상, 그 바탕인 호 길이 스캔, CPU·GPU 피킹이 모두 계단 경로를 따르므로 점선 계단선은 모서리마다 패턴이 이어지고, 수직선 위를 클릭하면 경로상 위치에 따라 두 원본 인덱스 중 가까운 쪽을 돌려준다.

패널 안의 그리기 순서는 "모든 시리즈의 채움 → 모든 시리즈의 본체 → 모든 선택 강조"이며, 각 단계는 설정에 선언한 시리즈 순서를 그대로 따르고 자동으로 정렬하지 않는다. 따라서 다른 시리즈의 채움이 이 시리즈의 선을 덮는 일은 구조상 없고, 채움끼리는 선언 순서대로 겹친다. 채움은 선의 속성이지 별도 히트 대상이 아니다. `pick_point` / `pick_data`와 선택 표시는 선의 표본을 기준으로 동작하며 시리즈를 선택하면 채움도 함께 선택된다. 채움이 있는 시리즈의 범례 기호는 같은 고정 너비 영역 안에서 선 아래에 채움색 띠를 그린다.

제약:

- 채움은 x가 단조인 시리즈를 전제한다. x가 되돌아가는 구간에서는 사다리꼴이 겹쳐 반투명 채움이 두 번 섞인다.
- 채움과 계단 보간은 모두 GPU 상주 컬럼이 필요하다. 비상주(스트리밍) 시리즈에 채움이나 계단 보간을 지정하면 `inspect_streaming_capability`와 스트리밍 준비 경로가 명시적 오류를 돌려준다. 청크 단위 스트림은 두 시리즈 사이 채움에 필요한 인덱스 정렬을 보장할 수 없다.
- 선의 끊김(유한하지 않은 표본, 로그축의 0 이하 표본)은 같은 자리에서 채움도 끊는다.
- 로그 y축에서 0 이하 기준선은 위치가 없다. 면 전체를 지우는 대신 축 최솟값으로 고정한다. 로그축의 0 이하 값을 생략하는 규칙의 명시적 예외다.
- 채움은 그리기 스타일이 그 선을 그릴 때만 그린다(별자리 스타일은 `ScatterLine` 시리즈만 그린다). 채움은 항상 정밀 기하로 그리며, 스케치·밀키웨이·별자리 선은 그 위에 각자의 획을 올린다.

히스토그램 막대는 구간 가운데에 배치하며, `width_ratio`로 구간 대비 너비를 `0..=1` 범위에서 정한다. 여기에 `gap_px`만큼의 픽셀 간격을 추가로 뺀다. 너비가 1픽셀보다 큰 막대는 최소 1픽셀이 남도록 간격을 제한한다.

구간 자체가 화면에서 1픽셀보다 좁으면 GPU가 픽셀 열별로 겹치는 구간 중 최댓값을 골라 0까지 채운다. 가로 히스토그램은 픽셀 행을 기준으로 계산하며 표시 범위를 벗어난 부분은 자른다. 이렇게 최댓값으로 고른 구간의 외곽선 두께와 불투명도가 모두 양수이면 그 영역 전체를 외곽선 색으로, 그렇지 않으면 채움색으로 칠한다. 최댓값이 같으면 앞선 구간을 고른다. 이 경로에서는 `gap_px`나 양수인 `width_ratio`로 틈을 만들지 않는다. 원본 컬럼은 유지하고 `width_ratio = 0`인 구간은 제외한다.

`border_width = 0`이면 외곽선을 숨기며, 양수이면 지정한 두께와 `border_color`를 적용한다. `bar_style_overrides`에는 `index`로 특정 구간을 골라 `fill_color`, `border_color`, `border_width`, `gap_px`, `width_ratio` 중 바꿀 값만 넣는다. 기준선과 방향은 시리즈 전체에 적용된다. 그리기, 데이터 피킹, 선택 테두리는 모두 같은 최종 막대 경계를 사용한다.

행렬 기반 세 종류는 등록된 컬럼 ID 묶음인 `MatrixRef { columns, orientation, grid_layout }`로 격자를 지정한다. 별도의 행렬 컨테이너를 만들지 않는다. 상주 경로에서는 풀의 컬럼을 읽고, 지원되는 비상주 히트맵 경로에서는 같은 ID의 원본 구간을 나누어 공급받는다.
`grid_layout`은 좌표가 셀 경계(`Edges`, n+1개)인지 중심(`Centers`, n개)인지 명시하며 배열 길이로 추측하지 않는다. 선언과 데이터의 크기가 다르면 공통으로 사용할 수 있는 범위까지만 그리고 잘림 여부를 알린다.

<!-- contour-contract: scope=config-ko max-levels=1024 -->
`ContourConfig.levels`에는 데이터 단위의 등고선 값을 명시적으로 나열한다. 렌더링 중 임의로 레벨을 만들지 않는다. `per_level_color: None`이면 모든 선에 `line.line_color`를 적용하며 색상표에서 색을 자동으로 고르지 않는다. 레벨 수는 `0..=1024`개다. 1025개 이상이면 오류를 반환하며 일부를 잘라서 그리지 않는다.

캐시에 없는 입력은 원본 목록과 선언 순서를 유지하면서 32개씩 묶어 정렬한 검색용 복사본을 만든다. 프래그먼트 셰이더는 최대 32개 블록을 이진 탐색해 해당 셀을 지날 수 있는 레벨만 계산한다. 한 셀에 1024개가 모두 걸리면 선언 순서대로 전부 합성한다. 선은 이중선형 보간한 면의 등값선이며, 점을 이어 만드는 대신 프래그먼트마다 그 면의 해석적 기울기로 그린다. 따라서 띠 경계와 그 위의 선은 같은 계산에서 나온다. 거리 계산에는 현재 셀의 보간 함수를 기울기 방향, 즉 등고선의 법선 방향 직선으로 제한해 얻은 이차방정식의 근을 사용한다. 여러 셀에 걸친 등고선 전체의 최단거리를 구하는 방식은 아니다.

유한하지 않은 레벨은 실제 업로드된 f32 값을 기준으로 처리한다. 등고선에서는 NaN과 양·음의 무한대를 모두 제외한다. `FillMode::Bands`의 분자는 음의 무한대 개수와 Z값 이하인 유한 레벨 개수를 더하고, 분모에는 선언한 전체 레벨 수를 사용한다.
`t=(negative_infinity_count + finite_le_z + 0.5)/(declared_level_count + 1)`이므로 NaN과 양의 무한대는 분모에만 영향을 준다.

`ContourLabelConfig.anchors`는 라벨 위치를 직접 지정할 때 사용한다. 기본값인 빈 목록에서는 GPU가 데이터 영역에 `spacing_px` 간격으로 후보를 만들고 각 후보를 해당 등고선 위로 투영한다. 투영된 후보는 도착한 셀 안에서 레벨을 실제로 가로지를 때만 남는다. 셀 꼭짓점 값의 범위가 레벨을 포함해야 하며, 이는 선을 그리는 조건과 같다. 따라서 레벨에 가깝기만 한 얕은 극값에는 라벨을 붙이지 않는다. 일반 선택에서는 이 간격을 목표로 후보를 남기지만, 레벨이 누락되지 않도록 추가로 고르는 후보는 더 가까울 수 있다.

`spacing_px`는 라벨을 숨겼거나 앵커를 직접 지정했더라도 항상 유한한 양수여야 한다. 자동 배치와 직접 지정 모두 최대 1024개를 사용한다. 직접 지정한 목록에서는 잘못된 `level_index`를 제외하고 유효한 앞 1024개만 남긴다. 이 결과가 비면 자동 배치하고, 하나라도 남으면 해당 목록으로 자동 배치를 대신한다.

출력 배율은 자동 배치 간격에만 곱한다. 출력 배율을 허용 범위로 제한한 뒤 그 곱도 유한한 양수인지 확인한다. 라벨 아틀라스는 어댑터의 텍스처 크기 한도를 지켜야 한다. 간격 오류, 곱셈 오버플로, 아틀라스 크기 초과가 발생하면 상태를 반영하기 전에 실패하므로 기존 차트와 GPU 자원을 유지한다. 앵커는 데이터 좌표와 데이터 공간의 접선으로 저장하므로 확대·이동 시 화면 좌표로 다시 투영할 수 있다.

라벨 글자색은 선 색이나 `per_level_color`와 별도로 `ContourLabelConfig.color`에서 정한다. 소수 표시는 아래쪽 X축이 아니라 등고선 레벨 간격을 기준으로 하며, 간격을 구할 수 없으면 색상 막대의 간격을 사용한다. `significant_digits`를 적용하되 인접 레벨이 같은 문자열로 표시되지 않도록 조절한다. 등고선 셰이더는 라벨을 그릴 때와 같은 앵커 버퍼를 읽어 라벨 사각형 안의 선을 그리지 않는다. `bg_color`가 없어도 `bg_padding_px`만큼 선을 더 비운다. `bg_color` 상자는 어떤 각도로 회전해도 가장자리가 부드럽다. 라벨 사각형이 상자 바깥의 투명한 아틀라스 여백까지 이어져, 상자 가장자리가 사각형 경계로 잘리지 않고 글자처럼 텍스처 필터링을 거친다.

`Renderer::series_draw_info(chart, series_id) -> Result<SeriesDrawInfo>`는 실제로 그린 데이터의 수를 알려 준다. 차트가 없으면 `UnknownChart`, 해당 id의 시리즈가 없으면 `InvalidSeriesConfig` 오류를 돌려준다. `drawn_count`, 행렬의 `cols` / `rows`, 잘림 여부인 `truncated`를 확인할 수 있다. 컬럼 길이가 다르면 공통 범위까지만 그리고 그 결과를 보고한다. 예를 들어 경계가 11개이고 빈도 값이 9개이면 막대 9개를 그린다. 선·산점도에서 X와 Y의 길이 중 작은 쪽까지만 그리는 경우도 이 API로 확인할 수 있다.

히스토그램과 행렬 기반 세 종류는 점 전용 피킹 API를 사용하지 않는다. 히스토그램의 자동 범위 맞춤은 업로드한 경계·값 메타데이터를 이용한다. 행렬은 GPU 범위 계산 엔진의 별도 모드를 사용한다. CPU는 잘림을 반영한 셀 수만 전달하고, GPU는 그리기와 같은 좌표 풀에서 `Edges` / `Centers` 및 셀·표본 격자 규칙을 적용한다. 등고선과 보간 채움은 표본의 양 끝에, 셀 단위 채움은 셀 경계에 맞춘다. `Edges`에서 표본 위치는 인접 경계의 중점이다. 선택은 `pick_data`의 막대·행렬 전용 셰이더가 담당한다.

**`Renderer::create_style_for_series(cfg)`**는 시리즈 설정에서 색·두께·모양을 읽어 화면용 `ChartStyle`을 만든다. 반환형은 `Result`다. 유니폼 4개와 선택적인 스타일 매핑 버퍼를 만들기 전에 남은 GPU 예산을 검사한다. 준비된 프레임이 바인딩을 보관하는 동안에도 메모리 집계는 유지된다. 출력용 `create_style_for_series_scaled(cfg, scale)`도 같은 검사를 거치며 픽셀 크기에만 배율을 적용한다.

**한 축 방향의 오차 막대만 그릴 때**(`ScatterErrorbarY` 등)는 `PrimitiveStyle::primitive_flags`의 Y=bit 0, X=bit 1로 방향을 지정한다. 사용하지 않는 정점 슬롯에는 이미 연결된 기준점 컬럼을 다시 바인딩하고, 셰이더는 오차 속성을 읽기 전에 해당 방향의 처리를 생략한다. 따라서 준비·출력 과정에서 빈 값으로 채운 숨은 컬럼을 만들지 않으며, 호스트도 그리지 않는 방향의 오차 컬럼이나 별도 메타데이터를 준비하지 않아도 된다. 오차값 0은 방향이 없다는 뜻이 아니라 길이가 0인 유효한 오차 막대다. `Symmetric`은 같은 오차 컬럼을 양쪽에 사용한다.

### `Config::scaled(scale)` / `Config::scale_in_place(s)`
폰트·선·여백 등 픽셀 단위 크기에 배율을 적용한다. `min/max/major_spacing`, 축 스케일 종류와 색은 바꾸지 않는다. 해상도와 관계없이 같은 모양으로 고해상도 이미지를 출력할 때 사용한다.

### 기본값 빌더(`renderer::default::default_config()`)
- `bottom_x` / `left_y`: 축선·눈금·라벨·제목을 켠다. 제목 텍스트는 비어 있다.
- `top_x` / `right_y`: 축선·눈금만 켜고 라벨·제목은 숨긴다. `out_margin = 8`로 좁은 여백을 둔다.
- `chart_title`: 표시를 켜고 `top_margin = 32`로 설정한다. 텍스트는 비어 있다.
- `grid`: 옅은 회색 주 격자선만 켠다.
- `legend`: 숨긴다.

빈 텍스트는 `Chart::with_title / with_x_title / with_y_title / with_legend_entry`로 채울 수 있다.

---
