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
and a progress fraction. The renderer draws them; it computes no running
total, change, duration or statistic. It decides only how the given values
look: it compares the two given ends of a bar to pick the up or down
colour, maps a state index to a style, places the bar in its slot and
shades the finished part of a bar. Streamed sources are rejected.

## Series

`x_column` holds the positions and `y_column` the end values, whatever the
direction (the box plot convention). Every other column is named in the
series:

| Field | Holds |
|---|---|
| `start` | the start values |
| `whiskers` | `RangeWhiskers { low, high, width, color }`: a stroke from the low to the high value at the bar's middle, under the body (a candle's wick) |
| `color_by` | `RangeColor::Single` (the series colour), `Direction { up, down, total }` (up where end ≥ start, down otherwise; a `RangeTotal { column, color }` flag column that reads 1 marks a total or subtotal) or `State { column, styles }` (the bar of state `k` takes `styles[k]`; a state with no style takes the series colour) |
| `progress` | `RangeProgress { column, remaining_opacity }`: the bar is whole up to `start + p · (end − start)`, `p` clamped to `0..=1`, and the rest is drawn at the remaining opacity; a NaN `p` draws the bar whole |
| `connector` | `RangeConnector { color, width }`: a line at each bar's end value, from its edge to the next row's bar (a waterfall) |
| `slot_width` | the slot of one row in data units on a numeric position axis (a time axis); `None` on a categorical axis, where the slot is one category |
| `style` | a `CategoryBarStyle`: material, corner radius (both ends of a floating bar are rounded), outline, texture, gloss and the brightness of a picked bar |
| `labels` | `CategoryBarSeriesLabels`: the host's strings, one per row (`text`), their placement (`Outside`, `Inside`, `Auto` = outside, `None`) and drag offsets; `decimals` and `value_suffix` are not read |

Every column holds one value per row of the positions; a column of another
length is an error naming it. A row with a NaN position, start or end, or
with equal ends, draws no bar and none of its parts. There is no limit on
rows, series, states or labels beyond the column pool and `u32`. The cost
grows with the rows and is the host's to budget: on the software Vulkan
adapter the tests run on, picking among 100,000 rows takes about 10 ms and
exporting them on a 7600 px wide panel about 1.8 s
(`three_thousand_rows_draw_pick_and_highlight` in
`tests/range_bars_render.rs` prints both).

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
use renderer::{Color, DataRenderType, SeriesConfig};

// A waterfall: the host's running totals before (`start`) and after
// (`end`) each step, a total flag column, connectors.
let waterfall = SeriesConfig {
    series_id: "balance".into(),
    source_id: None,
    label: None,
    x_column: "positions".into(),
    y_column: "end".into(),
    render_type: DataRenderType::RangeBars {
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
    },
};
```

## Interaction

Every part of a bar picks it: the body, the whisker and the connector each
answer `PickedData::HistogramBin` with the series and the row, from the
same functions the draw builds them with, so a bar is picked where it is
painted. The picked bar's highlight rings its body. `step_selection` moves
a bar along its rows by the delta of the pointer axis its position axis
runs along (`dx` for vertical bars, `dy` for horizontal ones) and across
the range bar series of its layer by the other; the rows are bounded by
the shortest of the series' columns. The web wrapper's
`selection_details()` reports the values the row's columns hold:
`position`, `end`, `start` and, when the series has them, `low`, `high`,
`total` or `state`, and `progress`, with `category_id` and
`category_label` on a categorical axis. It reports no difference or
duration: those are the host's to derive.

Value labels are picked, selected and dragged as a category bar's are
(`labels.offsets`), one per row. Range bars draw on overlays and subplots
with groups of their own, export to PNG at any scale and to SVG and PDF,
and serialize with every unset option omitted.

## Examples

The playground's Waterfall, Candlesticks and Gantt scenes
(`crates/renderer/tests/support/playground_scenes.rs`) build each chart;
the decoration oracle sweeps them like every other scene.
