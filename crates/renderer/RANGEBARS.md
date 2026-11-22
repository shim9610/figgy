# Range bars

A range bar floats from a start value to an end value at a position. One
series kind, `DataRenderType::RangeBars`, draws four charts that share
that shape:

| Chart | Position | Body | Whiskers | Colour | Extra |
|---|---|---|---|---|---|
| Waterfall | category | running total before → after | — | up / down / total | connectors |
| Candlestick | time | open → close | low → high | up / down | — |
| State timeline, Gantt | row | start → end (horizontal) | — | per state | progress |
| Floating range (min–max bars) | any | low → high | — | single | — |

The host supplies every value as a column: the running totals of a
waterfall, the prices of a candle, the bounds of an interval, a state index
and a progress fraction. The renderer draws them as given: it does not
compute running totals, changes, durations or statistics. It only compares a
bar's two ends to choose the up or down colour, maps a state index to a
style or a row index to a row colour, and shades the finished part of a
bar. Streamed sources are rejected.

## Series

`x_column` holds the positions and `y_column` the end values, whatever the
direction (the box plot convention). Every other column is named in the
series:

| Field | Holds |
|---|---|
| `start` | the start values |
| `whiskers` | `RangeWhiskers { low, high, width, color }`: a stroke from the low to the high value at the bar's middle, under the body (a candle's wick) |
| `color_by` | `RangeColor::Single` (the series colour), `Direction { up, down, total }` (up where end ≥ start, down otherwise; a `RangeTotal { column, color }` flag column that reads 1 marks a total or subtotal) `State { column, styles }` (the bar of state `k` takes `styles[k]`; a state with no style takes the series colour) or `ByRow { colors }` (row `i` takes `colors[i % colors.len()]`, a timeline's one colour per task; no colours draw the series colour; it reads no column) |
| `progress` | `RangeProgress { column, remaining_opacity }`: the bar is whole up to `start + p · (end − start)`, `p` clamped to `0..=1`, and the rest is drawn at the remaining opacity; a NaN `p` draws the bar whole |
| `connector` | `RangeConnector { color, width }`: a line at each bar's end value, from its edge to the next row's bar (a waterfall) |
| `slot_width` | the slot of one row in data units on a numeric position axis (a time axis); `None` on a categorical axis, where the slot is one category |
| `style` | a `CategoryBarStyle`: material, corner radius (both ends of a floating bar are rounded), outline, texture, gloss and the brightness of a picked bar |
| `labels` | `CategoryBarSeriesLabels`: the host's strings, one per row (`text`), their placement (`Outside`, `Inside`, `Auto` = outside, `InsideOrBeside`: inside when the label fits with the bar padding, else beside the bar's end, `None`), drag offsets and `contrast_ink: Option<[dark, light]>` (each label pixel in the ink that contrasts more with what lies under it, chosen on every draw: the bar's fill, its unfinished part past a progress split, or the plot background off the bar); `decimals` and `value_suffix` are not read |

Every column holds one value per row of the positions; a column of another
length is an error naming it. A row with a NaN position, start or end, or
with equal ends, draws no bar and none of its parts. There is no limit on
rows, series, states, row colours or labels beyond `u32` indices and device memory. The
cost grows with the rows: on a software Vulkan adapter
(lavapipe), picking among 100,000 rows takes about 10 ms and exporting them
on a 7600 px wide panel about 1.8 s.

## Layout and axes

The `RangeBars` series of one layer form its range group: they stand side
by side in series order (planned beside actual), laid out by the layer's
`Config::bar_groups`, whose mode must be `Grouped`. `Vertical` stands the
bars up from a position axis on `bottom_x` (a waterfall, candles);
`Horizontal` lays them along x from a position axis on `left_y` (a Gantt
chart). The position axis is categorical or linear numeric; the value
axis is numeric, linear or logarithmic (prices). A layer draws either
category bars, box plots or range bars. A connector needs its series alone
in its layer, and horizontal range bars cannot sit on an overlay. A polar
chart draws no range bars.

`Chart::auto_fit_layer_union` fits the value axis to the start and end
columns and, with whiskers, to the low and high ones, without forcing zero
(a bar floats); on a numeric position axis it fits the positions grown by
half a slot on each side. A time axis keeps its precision: upload seconds
with `Renderer::add_hilo_column` to keep the full `f64`.

```rust
use renderer::data_config::{
    BarGroups, CategoryBarSeriesLabels, CategoryBarStyle, RangeColor, RangeTotal,
    RangeConnector,
};
use renderer::{Color, DataRenderType, RangeBarSeries, SeriesConfig};

// A waterfall: the host's running totals before (`start`) and after
// (`end`) each step, a total flag column, connectors.
let waterfall = SeriesConfig {
    series_id: "balance".into(),
    source_id: None,
    label: None,
    x_column: "positions".into(),
    y_column: "end".into(),
    z_column: None,
    render_type: DataRenderType::RangeBars(RangeBarSeries {
        color: Color::from_rgb8(90, 110, 130),
        style: CategoryBarStyle::default(),
        start: "start".into(),
        whiskers: None,
        color_by: RangeColor::Direction {
            up: Color::from_rgb8(44, 160, 90),
            down: Color::from_rgb8(214, 72, 64),
            total: Some(RangeTotal {
                column: "is_total".into(),
                color: Color::from_rgb8(65, 110, 180),
            }),
        },
        progress: None,
        connector: Some(RangeConnector {
            color: Color::from_rgb8(110, 110, 110),
            width: 1.5,
        }),
        slot_width: None,
        labels: CategoryBarSeriesLabels::default(),
    }),
};
```

## Timelines

`SeriesConfig::timeline_bars(id, positions, start, end)` and
`default::default_timeline_config()` (both with the `helpers` feature)
build a timeline in the look of the common web timelines (#357), and
`default::config_for_series` gives a chart whose series hold a range link
series (a Gantt chart's dependencies) this frame too:

| Choice | Value | References |
|---|---|---|
| Category order | the first row at the top: `left_y.inverted` | Vega-Lite, Google Charts, Frappe and DHTMLX list the first row at the top; Plotly's `px.timeline` needs `autorange="reversed"` for it |
| Rows | a faint band behind every other row (`left_y.category_bands`, `#F7F8FA`), no row axis line, task names in dark slate (`#374151`) | Google Charts' `alternatingRowStyle` (on by default); the project tools draw rows as a table |
| Colour | one muted colour per row (`RangeColor::ByRow` over `default::TIMELINE_COLORS`: soft blue, teal, violet, amber, rose, slate, green, sky); the series colour is the first | Google Charts colours each bar; no reference uses a saturated categorical cycle per row |
| Bar thickness | 0.6 of the row's band (`bar_groups.group_width`) | Frappe, DHTMLX and the project tools fill about 60 % of a row |
| Corner radius | 4 px, flat, no texture or gloss | the web Gantt libraries round 3 to 6 px |
| Labels | inside the bar when they fit, else beside its end (`InsideOrBeside`), in dark slate or white, whichever contrasts more (`contrast_ink`, `default::TIMELINE_LABEL_INK`) | Frappe, DHTMLX and Bryntum put the task name in the bar and outside a short one |
| Progress | `SeriesConfig::with_progress(column)`: the host's share, the unfinished part at 0.4 opacity (`default::TIMELINE_REMAINING_OPACITY`) | the same hue, lighter past the done share |
| Gridlines | time gridlines only, pale grey (`#E8EBF0`), no tick marks, the time labels in muted grey (`#6B7280`), no bottom axis line | every reference grids the time axis only |
| Frame | a light top rule (`#D1D5DB`), no other frame | the web Gantt charts draw no box |
| Layout | `LayoutState::Fitted`: the chart is created with its margins fitted to the row labels | every reference sizes the label band to its labels |

A web Gantt's other elements are the host's, with existing parts: a
milestone is a `ScatterShape::DiamondFilled` scatter series on the bars'
layer (positions the row index, x the date; no row of the bars needs a
value), today is an `AnnotationKind::ReferenceLine` on `X` at the time
the host passes (the renderer never reads a clock), and finish-to-start
links are a `RangeLinks` series (below). The `timeline` scene of
`examples/range_bars.rs` draws all of them.

Every choice is a stored field, so a saved timeline reads back with the
same look, and the documented defaults (`SeriesConfig::range_bars`,
`default_config`, the serde defaults) are unchanged. A theme recolours a
row-coloured series from its palette and draws no gridlines along a
categorical position axis; the gallery's range bars take the open frame
without tick marks.

```rust
use renderer::{Category, Chart, SeriesConfig};

let mut config = renderer::default::default_timeline_config();
config.left_y.categories = Some(vec![
    Category::new("scope", "Scope"),
    Category::new("build", "Build"),
]);
let chart = Chart::new(config).with_area(0, 0, 800, 600);
let tasks = SeriesConfig::timeline_bars("tasks", "row", "start", "end").with_progress("done");
```

### Dependency links

`SeriesConfig::range_links(id, bars, [(from, to), …])` adds a
`DataRenderType::RangeLinks` series: each `RangeLink { from, to,
exit_fraction }` joins the bar of row `from` to the bar of row `to` of the
range bar series `bars`, finish to start. The series is DataLess: the links
are SSOT (`links`), like annotations, and it reads no column (`x_column`
and `y_column` empty, `RuleKind::RangeLinkColumns`). There is no cap on
links; each costs one 6-vertex instance and one pick candidate per draw,
and 12 bytes of the lane table the series uploads when it is prepared
(`three_thousand_links_draw_and_pick` draws 3749). The renderer draws each
link on every draw from the two bars as they are drawn, in the web Gantt
convention (Frappe Gantt): the link leaves the predecessor's edge facing the
successor's row at its exit, runs across to the successor's centre line,
turns through one rounded corner and ends in an open chevron just before the
successor's start. Links leaving one bar at one exit share their trunk, and
a link between two tasks on one row runs straight from the first's end. A
link naming a row past the bars' rows draws and picks nothing, and
`series_draw_info` reports the series as `truncated`. A link series draws
just before its bars, whatever its place in the series list
(`data_config::series_draw_order`), so the bars cover the link ends; the ink
mask, the pick's paint order and every export follow that order.

The layout chooses each link's exit and stores it (#357): `fit_ranges`, a
`Fitted` chart's registration and `commit_auto_fit_all` (in the browser
`fit_ranges()`, `fit_layout()`, `auto_fit_all()` and a new chart's first
fit) write an exit into every link whose `exit_fraction` is `None`: toward the
predecessor's end, `inset` short of it (half a bar shorter than two insets),
pulled back only as far as the successor's start needs room for the last
leg and the head, and kept on the bar
(`RangeLinkSeries::chosen_exit_offset`). The exit is stored as a fraction
`0..=1` of the predecessor's drawn length, from its start (0) to its end
(1), so it keeps its place on the bar when the chart zooms, resizes or
exports at another scale. The draw maps the fraction onto the bar's
current drawn span and keeps the room rule: when the successor starts too
soon for the last leg and the head, the exit moves back. A value that is
already `Some` is never rewritten, whoever wrote it, so a dragged exit
survives later layouts; clearing one to `None` lets the next layout choose
again. A link without an exit (no layout has run) leaves `inset` logical
pixels from the predecessor's start, under the same room rule.
`Chart::fit_layout` holds no series: a host that lays out with it calls
`Renderer::fit_link_exits(config, &mut series)` afterwards.

The bars' drawn ends come from the extent engine's GPU readback: one
gather (`gather_link_ends`) reads each link's predecessor and successor
start and end from the column pool into a 48-byte record, written straight
into mapped memory and read back once by the layout; the CPU keeps no row
values. There is no cap on links: more links than one gather holds (one
storage binding) run several gathers. The cost grows with the links:
`many_links_lay_out_draw_and_pick` lays out 3749 links in about 0.1 s on
lavapipe. In a browser, which cannot wait, the gather is submitted at the
layout and the frame that finds it complete writes the exits
(`Renderer::service_link_exit_fits`); the series edit redraws the chart.

| Field | Default (`range_links`) | Range |
|---|---|---|
| `bars` | the bars' `series_id` | a range bar series in the same layer (`RuleKind::RangeLinkBars`) |
| `color` | amber `#D4A24C` | channels in `0..=1`; a theme keeps it, since the waterfall connector's thin dark line does not read as a dependency |
| `width` | 1.5 px | `0..=24` |
| `head` | 6 px open chevron | `0..=32` |
| `inset` | 6 px: how far short of the predecessor's end the layout puts an exit, and from its start a link without one leaves | `0..=64` |
| `links[i].exit_fraction` | `None`: chosen by the next layout | a fraction `0..=1` of the predecessor's drawn length from its start |

The style ranges are `RuleKind::RangeLinkStyle`. Polar charts refuse links
(`RuleKind::PolarRangeLinks`), and a streamed source is refused
(`RuleKind::StreamedRangeLinks`). The legend draws a link series as a line.

```rust
use renderer::SeriesConfig;

let tasks = SeriesConfig::timeline_bars("tasks", "row", "start", "end");
let links = SeriesConfig::range_links("links", "tasks", [(0, 1), (1, 2), (1, 3)]);
// In either order the bars cover the link ends; a layout
// (`Renderer::fit_ranges`) chooses each link's exit.
let series = [tasks, links];
```

## Interaction

Every part of a bar picks it: the body, the whisker and the connector each
answer `PickedData::HistogramBin` with the series and the row, so a bar is
picked where it is painted. The picked bar's highlight rings its body. `step_selection` moves
a bar along its rows by the delta of the pointer axis its position axis
runs along (`dx` for vertical bars, `dy` for horizontal ones) and across
the range bar series of its layer by the other; the rows are bounded by
the shortest of the series' columns. The web wrapper's
`selection_details()` reports the values the row's columns hold:
`position`, `end`, `start` and, when the series has them, `low`, `high`,
`total` or `state`, and `progress`, with `category_id` and
`category_label` on a categorical axis. It reports no difference or
duration: those are the host's to derive.

A link picks as `PickedData::HistogramBin` with its own series and its
index in `links`, wherever its stroke or head is painted; the picked link
is redrawn in the selection colour with a wider stroke, and
`step_selection` moves along the links by `dx + dy`, staying in the
series. `selection_details()` reports `index`, `from`, `to` and
`exit_fraction`. Links draw on overlays and subplots next to their bars and
export with them.

A link's exit is draggable. `RangeLinkSeries::drag_link_exit(index,
along, length)` moves it along the predecessor's edge by `along` logical
pixels on a predecessor drawn `length` logical pixels long: the stored
fraction moves by `along / length` and stops at 0 and 1, so dragging back
past the bar's end moves the exit at once. Every link that leaves the same
bar at the same exit moves with it, so a shared trunk moves as one, and an
exit within 3 px of `inset` snaps back to `None`, which the next layout
chooses again. The CPU holds no row values, so the length comes from the
link's pick: a range link hit (`PickedData::HistogramBin`) carries
`extent_px`, its predecessor's drawn length in display pixels (a range bar
hit carries its own), which the host divides by the display scale. Without
a length the drag cannot be converted and is rejected.
`set_link_exit_offset(index, fraction)` sets one link's exit directly,
`0..=1`. In the browser, `press_range_link(series_id, index, extent_px)`
after a `pick_data` hit (`hit.extent_px`) starts the drag, `on_move` moves
the exit along the value axis as it runs on screen (an inverted axis
included) and `on_release` ends it; `set_link_exit_offset` is the twin of
the Rust helper.

Value labels are picked, selected and dragged as a category bar's are
(`labels.offsets`), one per row, wherever their placement puts them
(inside, beside, or flipped back over the bar at the plot's edge); a
label in contrast ink keeps reading what lies under each of its pixels
after a drag. Range bars draw on overlays and subplots
with groups of their own, export to PNG at any scale and to SVG and PDF,
and serialize with every unset option omitted.

## Examples

[range_bars.rs](examples/range_bars.rs) draws a waterfall, a candlestick
chart and a Gantt chart (`gallery-waterfall.png`, `gallery-candlestick.png`,
`gallery-gantt.png`). The playground's *Waterfall (range bars)*,
*Candlesticks (range bars)* and *Gantt (range bars)* scenes build each chart
interactively.
