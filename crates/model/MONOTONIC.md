# Monotonic-x lines / 단조 x 선

A monotonic-x line chart draws large, evenly spaced signals, such as a data
acquisition recording or a live sensor feed, as lines at display
resolution. All channels of a group share one x grid: sample `i` is at
`x = start + i × interval`, with `interval > 0`. For each pixel column the
chart keeps four of the column's samples, the first, the lowest, the
highest and the last (the **M4** reduction), and draws the line through
them. The image looks like a line drawn through every sample (see
[Accuracy](#accuracy)), and the cost of a frame depends on the plot width
and the number of channels, not on the number of samples.

The chart draws lines only. A scatter plot would lose the points between
the four kept samples of a column, so it is not offered.

Your samples stay in your memory. You lend them, as a `MonotonicSource`, to
each call that reads them, and the chart holds no reference to them after
the call returns. What the chart keeps for each group is its **envelope**:
the kept points of every pixel column of every channel.

The source types and the envelope are in `model::monotonic` (re-exported by
`figgy-renderer` as `renderer::monotonic`). The chart calls are on the
renderer's `MonotonicChart`. The browser product offers the same calls on
typed arrays: see
[Monotonic-x lines in WASM.md](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#monotonic-lines).

## Quick start

```rust,ignore
use renderer::renderer::{MonotonicGroupRef, MonotonicRange};
use renderer::{MonotonicAppend, MonotonicSlices, SeriesConfig};

// One series per channel; both read group "daq".
let series = vec![
    SeriesConfig::monotonic_line("ch0", "daq", 0).label("channel 0"),
    SeriesConfig::monotonic_line("ch1", "daq", 1).label("channel 1"),
];
let id = renderer.register_monotonic_chart(config, series)?;

// The full call: the whole data with its grid. `Autoscale` fits both axes.
let channels = [ch0.as_slice(), ch1.as_slice()];
let source = MonotonicSlices::new(0.0, 1e-3, &channels);
let mut daq = renderer.monotonic_chart(id)?;
daq.set_monotonic("daq", &source, MonotonicRange::Autoscale)?;

// New samples only. `sliding` keeps the window's length and scrolls it.
let fresh = [new0.as_slice(), new1.as_slice()];
daq.append_monotonic("daq", &MonotonicSlices::new(0.0, 1e-3, &fresh),
    MonotonicAppend { sliding: true })?;

// A zoom is a full call at the new x range.
daq.set_monotonic("daq", &source, MonotonicRange::X { min: 0.0, max: 2.0 })?;

// A resize or a layout fit takes the groups' sources too.
let groups = [MonotonicGroupRef::new("daq", &source)];
daq.resize(chart_area, None, Some((width, height)), &groups)?;
```

[USAGE.md](https://github.com/shim9610/figgy/blob/master/crates/renderer/USAGE.md#monotonic-x-lines)
has the same example with the editor, drawing and export.

The playground in `tools/playground` has a worked example, the scene
"Monotonic x (recorded channels)": four channels of 200 000 samples each.
The scene keeps its samples and lends them on every call that takes a
source.

## Lending the samples

A `MonotonicSource` describes one group: its grid (`start()`, and
`interval()`, finite and positive), `len()` samples in each channel, and
`channel_count()` channels. It is borrowed for one call. Two built-in
sources cover the usual layouts:

| Source | Channel `n`, sample `i` |
|---|---|
| `MonotonicSlices::new(start, interval, &[&[T]])` | `slices[n][i]` |
| `MonotonicInterleaved::new(start, interval, values, stride, columns)` | `values[i * stride + columns[n]]` |

- `MonotonicSlices` takes one slice per channel. When the slices differ in
  length, it lends their common length and the call reports `truncated`.
- `MonotonicInterleaved` takes one sample-major buffer with `stride` values
  per frame, and channel `n` reads frame column `columns[n]`. Columns that
  no channel names, such as a timestamp, are skipped. A trailing partial
  frame counts once it is complete. `new` returns a `Result`: it fails with
  `InvalidStride` for a zero stride and with `InvalidColumn` for a column
  outside the frame.
- The element type `T` is `f64`, `f32`, `i32`, `i16`, `u16` or `u8`, so raw
  integer samples can be lent as they are, without converting them to
  `f64` first.
- A NaN or infinite float is a gap in the line. Integers have no gaps.

For another layout, such as a ring buffer, implement `MonotonicSource`
yourself. `lend(range, visit)` passes the requested samples to `visit` as
contiguous runs (`MonotonicRun::slices` or `MonotonicRun::interleaved`), in
order, with no gaps or overlaps. A ring buffer lends a wrapped range as two
runs.

## The chart

Register a chart with monotonic series through
`Renderer::register_monotonic_chart(config, series)`. It returns a
`MonotonicChartId`, and `Renderer::monotonic_chart(id)` lends a
`MonotonicChart` with the chart's calls. The chart may hold ordinary
series too.

Each channel you draw is its own series, with its own id, label, colour,
style and legend entry. `SeriesConfig::monotonic_line(id, group, channel)`
builds one: a `DataRenderType::MonotonicLine(MonotonicLineSeries { group,
channel, line })`. Series that name the same group share its source; a
series draws channel `channel` of it, counted from 0. In JSON the series is
`{"MonotonicLine": {"group": .., "channel": .., "line": {..}}}`.

The document never holds the samples. A series' `x_column` and `y_column`
name the two pool columns the chart draws its envelope from, and the helper
names them `<id>.monotonic.x` and `<id>.monotonic.y`. Only the chart writes
them: writing or removing one yourself fails with `ReservedColumnId`.

The document must suit the reduction, or the chart refuses it:

- The line is linear (`line.interpolation` is `Linear`) and has no fill. A
  monotonic line has no markers.
- The x axis is linear, numeric and unbroken. A logarithmic, probability,
  categorical or broken x axis is refused.
- The chart is a rectangular 2D chart. Polar, ternary, parallel and 3D
  charts are refused.

The general chart calls (`set_chart_config`, `set_chart_state`,
`set_chart_view_state`, `set_chart_display`, `hold_chart_display`, the
auto-fit calls, streaming and the rest) take a `ChartId`. A monotonic chart
never hands one out, so the compiler rejects any attempt to apply them to
it, and monotonic charts do not appear in `chart_order()`. In the other
direction, `register_chart`, `set_chart_series`, `set_chart_state` and
`begin_replace_chart_columns` refuse a monotonic line series
(`MonotonicGeneralChart`).

## Calls

The **x-to-pixel mapping** decides which pixel column each sample falls in:
the x range, the plot's width in device pixels, and the x scale, breaks and
direction. Since the chart holds only the envelope, a change of the
mapping needs the samples again. Four calls change it, and each takes the
source:

| `MonotonicChart::` | Source | What it does |
|---|---|---|
| `set_monotonic(group, &source, range)` | the whole data | Reduces the group from scratch and sets its x range. |
| `append_monotonic(group, &new, MonotonicAppend { sliding })` | the new samples only | Adds samples; with `sliding`, scrolls the window. |
| `resize(chart_area, plot_area, surface, &[MonotonicGroupRef])` | each moved group's | Resizes the chart, the plot or the display. |
| `fit_layout(&[MonotonicGroupRef])` | each moved group's | Fits the layout to the labels and titles. |

Everything else works without the source: the document editor
(`edit_config`), `set_series` (a style, a channel, a group, series added or
removed), `step_selection`, `set_selection`, `pick_data`, `pick`,
`prepare_picking`, `remove`, drawing, and an export at the screen's scale.
The envelope is in data units, so changing the y range or y scale, colours,
the line's width and style, titles, ticks, the legend or annotations needs
no source.

### `set_monotonic`: the full call

`set_monotonic(group, &source, range)` reduces every series that names
`group` from scratch, at the mapping its panel is drawn at, and is the only
call that sets the x range. `range` is one of:

- `MonotonicRange::X { min, max }`: this x range, written to the x axis the
  group's series are drawn against. The rest of the axis and the y range
  stay as they are. A zoom or a pan is a full call with the new range.
- `MonotonicRange::Autoscale`: the x range is set to the samples' span,
  `start ..= start + (len - 1) × interval`, with the tick plan an auto-fit
  uses. Each y axis the group draws on is fitted to the lowest and highest
  values that every monotonic series drawn on it keeps in the plot, of this
  group and of any other, padded by 5 % of the span on each side, so two
  channels, or two groups, on one axis both stay in view. A
  series on the base layer fits the panel's `left_y` (and its
  partner axis), an overlay series fits its overlay's own axes (its own x
  axis too while the overlay's `link_x` is off; the panel's `left_y` while
  its `link_y` is on), and a subplot series fits its subplot's axes (its x
  axis through the root of its link chain). An extent with no width, such as one sample or a
  constant channel, leaves its axis as it is. The general auto-fit calls do
  not reach a monotonic chart, so this is how you fit one.

The call reads only the samples in view, 64 pixel columns' worth beyond
each edge of the plot, and one sample on each side of that, so zooming into
a small part of a long recording costs only what is in view. The sample
indices the chart reports (in picks and selections) count from the start of
this source.

### `append_monotonic`: new samples

`append_monotonic(group, &new, MonotonicAppend { sliding })` takes only the
new samples, every channel the same count. Sample `j` of `new` becomes
sample `len + j` of the group, at `start + (len + j) × interval`, where
`len` is the number of samples the group holds. The new samples continue
the grid of the last full call: `new`'s own `start` and `interval` are not
read, and the x scale never changes. An append never reads earlier samples,
so you can drop old data between full calls; the next call that takes the
source reduces from whatever source you lend it.

- Without `sliding`, the window stays where it is. New samples inside it
  are drawn. Samples beyond its right edge are counted but not kept, except
  the first one, which the line runs to.
- With `sliding: true`, the window keeps its length and the x range moves
  by exactly `new.len() × interval`, which the call writes to the x axis
  (the panel's `bottom_x`, the overlay's own `x_axis` for a group on an
  overlay with `link_x` off, or a subplot's own x axis at the root of its
  link chain). The pixel columns stay on the grid of the last full call:
  columns that leave the window on the left are dropped and new samples
  join at the right. A slide by part of a column therefore differs from a
  full call by less than one pixel column until the next full call
  realigns it.
- A slide fails with `SlidePastKept` when it would bring into view samples
  that were counted but not kept, that is, samples an earlier append
  without `sliding` placed beyond the right edge. Make a full call with
  the source instead.

An append before the group's first full call fails with `NotBuilt`, and an
append with another channel count fails with `ChannelCountChanged`.

### `resize` and `fit_layout`

`resize(chart_area, plot_area, surface, &groups)` sets the chart area, the
plot (data) area a plot-area resize proposes, in document pixels (`None`
keeps the margins), and the display surface in device pixels (`None`: no
display). `fit_layout(&groups)` fits the layout to the labels and titles,
measured with the chart's own font, which can move the plot's edges.

`groups` lends sources as `MonotonicGroupRef::new(group, &source)`. When
the call changes the chart, every group whose source you pass is reduced
again from it. A group you leave out keeps its envelope if its series stay
on the same device pixels; if they move, the call fails with
`FiggyError::MonotonicSourceRequired { series_id, change }`. Passing every
group's source is always safe. A call that changes nothing reads no source.

Both calls are atomic: a refusal changes nothing, neither the document nor
the display nor the image. A plot rect the margins cannot place is refused
with `NoFeasibleLayout`.

### Editing the document

`edit_config(|editor| ..)` lends a `MonotonicConfig` over a copy of the
document. `config()` reads the whole `Config`, and the editor changes
everything that leaves the mapping alone: the legend, the chart title, the
grid, annotations, the data selection (`picked_data`), the active layer,
the backgrounds, each axis (`MonotonicAxis`: title, tick labels and their
format, ticks, spacing, axis line, and a y axis' range), overlay axes
(`overlay_axis`, and `overlay_x_axis` for an overlay's own x axis, whose
range is the monotonic calls') and legends, and subplots under the same
rules. It has no setter for the x
range, x scale, breaks, direction, chart area or plot area.

Decoration drags go through `nudge` and `drag`, which reject a move that
would change the mapping, such as an axis or plot edge that changes the
plot's width, or a subplot frame. `assign(config)` replaces the whole
document, for a document edited as data, and refuses one that changes the
mapping. When the edited copy changes the mapping anyway, the edit fails
with `MonotonicMappingEdit` and changes nothing.

### Reading, drawing and exporting

- Reads take `&Renderer`: `monotonic_config`, `monotonic_series`,
  `monotonic_selection`, `monotonic_display_config`,
  `monotonic_render_stamp`, `monotonic_series_draw_info`,
  `monotonic_kept_point` (a kept point's `(x, y)` by sample index), and
  `monotonic_needs_source`, which lists the series whose group has never
  had a full call (`MappingChange::NotBuilt`). Those series draw nothing
  until `set_monotonic`.
- `Renderer::prepare_monotonic` and `WindowedRenderer::draw_monotonic` draw
  `MonotonicChartDrawItem`s. A frame never needs the source.
- `ExportSource::Monotonic { chart, monotonic: &groups }` exports the chart
  as PNG, SVG or PDF. At the screen's mapping, pass `&[]`. At another
  mapping (a 2x export, or a 1x export of a chart displayed at another
  scale), pass the sources: each group is reduced at the export's mapping
  for that export only. A series whose source is missing fails the export
  with `MonotonicSourceRequired`.

Every call that reduces returns a `MonotonicUpdate`: `len` (samples the
group holds), `truncated`, `dropped` (columns a slide dropped), `x_range`
(the x range after the call) and `pixel_offset` (how far, as a fraction of
a column in `0..1`, the window has slid past the column grid; the chart
draws the columns shifted by it).

## Cost

- **Per frame**, each channel draws `8 × (plot width + 131)` rows of the
  line pipeline, whatever the sample count: four points per pixel column,
  each followed by a join row, over the plot and its 64-column margin on
  each side. About half of the rows draw nothing.
- **Memory**: per group, `channels × (plot width + 131)` column records on
  the CPU for each mapping its series are drawn at (a subplot of another
  width adds one); per series, two pool columns of
  `8 × (plot width + 131)` rows. None of it grows with the sample count.
  The plot width here is in device pixels.
- **A full call** reads the samples in view plus the margins, once each.
- **An append** reads only the new samples.

## Accuracy

Compared with a chart that draws the same samples as an ordinary line:

- **Sparse views**, at most 4 visible samples per pixel column, keep every
  sample, and the images are byte-identical at 1x and 2x.
- **Dense and mixed views** draw each column's span through its first,
  lowest, highest and last samples. Every pixel either image paints lies
  within one pixel of a pixel the other paints, at 1x and 2x, for hairlines
  and thick lines. The images are not byte-identical: the full draw stacks
  one antialiased stroke per sample, and each stroke's fringe reaches the
  neighbouring column from wherever its sample lies.
- **Appends and whole-column slides** draw byte for byte as a full call on
  the same window. **Slides by part of a column** stay within one pixel of
  the full draw.
- **A gap narrower than one pixel column** is drawn joined: it cannot be
  resolved at display resolution. A wider gap breaks the line, and zooming
  in until a gap spans a column shows it.
- Kept points lie exactly where a full draw places them, slid or not.

## Interactions (M6)

- **Picks** (`pick`, `pick_data`, hover and click selection) report the
  sample index of the kept point at the nearer end of the drawn segment
  under the pointer, as an ordinary line's pick does. A dense column's
  drawn pixels come from exactly its kept points.
- **The picked highlight** rings the kept point with the selected sample
  index. A sample the envelope does not keep has no ring.
- **`step_selection`** steps through the kept points.
- **The legend** shows a line swatch in the series' colour, and a theme's
  palette recolours it.
- **Layers**: overlays and subplots (their own x range and width) both
  work. An overlay follows the parent's x axis by default; with its
  `link_x` off it has an x axis of its own, so a group recorded on another
  clock or at another rate shares the plot and each group's full call,
  append and Autoscale move only its own x axis (the playground's
  "Monotonic x (two clocks)" scene). Groups that share one x axis are moved
  together, so a call that moves it needs every group's source.
- **Decorations and the plot**: dragging the title, legend, axis titles,
  tick labels or the plot goes through the editor's `nudge` and `drag`. A
  plot-area resize is a `resize` with a plot rect, so it takes the
  sources.
- **Exports**: PNG, SVG and PDF at any scale, with the sources when the
  mapping differs from the screen's.
- **Streaming**: a monotonic series in a streamed display is refused
  (`StreamedMonotonic`).

## Errors

Errors from the reduction arrive as `FiggyError::Monotonic { group, error }`
with a `MonotonicError`:

| `MonotonicError` | Cause |
|---|---|
| `InvalidGrid { start, interval }` | `start` is not finite, or `interval` is not finite and positive. |
| `InvalidMapping { .. }` | The x range is not finite and increasing, or the plot width is zero. |
| `UnsupportedXMapping` | The x axis is not linear, or it is broken. |
| `InvalidStride`, `InvalidColumn { column, stride }` | `MonotonicInterleaved::new` got a zero stride or a column outside the frame. |
| `NotBuilt` | An append before the group's first full call. |
| `ChannelCountChanged { expected, found }` | An append with another channel count; make a full call. |
| `SlidePastKept` | A slide would show samples that were counted but not kept; make a full call. |
| `CountOverflow` | The group's sample count would overflow `usize`. |
| `OutOfMemory` | The envelope's memory could not be allocated. |
| `SourceRequired { change }` | `MonotonicEnvelope::check_mapping` found another mapping (core API only). |

The chart's own refusals follow. Those named by a `RuleKind` carry it in
the error's `kind` field.

| Error | Cause |
|---|---|
| `FiggyError::UnknownMonotonicGroup { group }` | No series of the chart names the group. |
| `FiggyError::MonotonicSourceRequired { series_id, change }` | A resize, layout fit or export would move a series whose group's source was not passed. Nothing changes. |
| `FiggyError::ReservedColumnId { id }` | A host write or removal of an envelope column. |
| `RuleKind::MonotonicRange` | `MonotonicRange::X` with a range that is not finite and increasing. |
| `RuleKind::MonotonicMappingEdit` | An editor change that would move the mapping. |
| `RuleKind::NoFeasibleLayout` | A resize's plot rect the margins cannot place. |
| `RuleKind::MonotonicChannel { channel, channels }` | A series names a channel the group's source does not have. |
| `RuleKind::MonotonicColumnTaken { column }` | An envelope column id already belongs to a host column, another series or another chart. |
| `RuleKind::MonotonicGeneralChart` | A monotonic series given to a general chart call. |
| `RuleKind::StreamedMonotonic` | A monotonic series in a streamed display. |
| `MonotonicXAxis`, `MonotonicChartKind`, `PolarMonotonic`, `MonotonicInterpolation`, `MonotonicFill`, `MonotonicGroupEmpty`, `MonotonicColumns`, `MonotonicColumnsShared` | The document breaks a rule of [The chart](#the-chart): the x axis, the chart kind, the line, an empty group name, or the envelope column ids. |

## Limits

- **Stroke width.** The envelope reaches 64 pixel columns beyond each edge
  of the plot, at the scale it is drawn at. Strokes up to about 120 device
  pixels wide (the width times the display or export scale) draw their
  edges as a full draw does; a wider stroke may miss ink at the plot's
  edges. Changing the width is a plain style edit and needs no source.
- **x axis.** Linear, numeric and unbroken only, on a rectangular 2D chart.
- **Lines only.** No scatter, no markers, no fill, no step interpolation.
- **No gestures that move the mapping without a source.** In the browser,
  wheel and drag zoom are off for monotonic charts, and a resize or
  plot-area resize waits for the host to pass the sources.
- **No streaming.**
- **No count limits** (M7).

## Using the core directly

`MonotonicEnvelope` is the reduction on its own, for a host that draws the
envelope itself. It works without the renderer.

- `MonotonicEnvelope::empty()` makes one. `set_monotonic(&source,
  &mapping)` is the full call, and `append_monotonic(&new,
  MonotonicAppend { sliding })` the append, as above. Both return a
  `MonotonicUpdate`.
- `MonotonicMapping` holds the x range, the plot width in device pixels,
  the x scale, breaks and inversion, and `margin_px`, the whole pixel
  columns kept beyond each edge of the plot.
  `MonotonicMapping::linear(x_min, x_max, plot_width_px)` builds a plain
  one with no margin. Sample `i` falls in plot column
  `floor((start + i × interval - x_min) / x_per_pixel)`, where
  `x_per_pixel = (x_max - x_min) / plot_width_px`.
- `check_mapping(&current)` fails with `SourceRequired { change }` when
  `current` differs from the mapping the envelope was built for, slides
  included. The envelope cannot rebuild itself: make a full call.
- `is_grid_aligned()` says whether the columns are aligned to the current
  x range (no pixel offset). `aligned_mapping()` is the x range they are
  aligned to.
- Each channel has `slots()` slots, `w + 3` where `w` is the plot width plus
  twice the margin: the lead edge sample (the last sample left of the
  columns), the columns `0..w`, column `w` (kept in full so that a slide by
  part of a column can show it), and the trail edge sample (the first
  sample right of column `w`). `cell(channel, slot)` returns an
  `EnvelopeCell` of up to 4 points, each with its value and sample index.
- To draw a channel, join point `k` of a cell to point `k + 1` where
  `cell.linked(k)` is true, and join the last point of a cell to the first
  point of the next non-empty cell where `EnvelopeCell::joins` is true
  (both finite, consecutive sample indices).
- `column_of_x(x)` gives the column under an x, and `pick(channel, column,
  y)` the kept point nearest to `y`, as its sample index and value.

## Guarantees (M1–M7)

What monotonic line series promise. Other documents refer to these as
M1–M7.

- **M1. Input.** Samples enter only through a `MonotonicSource`, which
  carries the values, `start` and `interval`, and is passed to every call
  that reads them. No call takes `start` or `interval` separately. You
  never upload the samples as columns.
- **M2. Reduction.** Each pixel column is reduced to its first, lowest,
  highest and last samples. After a full call or a whole-column change,
  each column covers the same span as drawing every segment, and a sparse
  view draws the same image as a full draw. While sliding by fractions of a
  column, the image differs by less than one pixel column until the next
  full call realigns it. A gap narrower than a column is not shown.
- **M3. Memory.** The chart keeps only the envelope (at most 4 points per
  column, sized by the plot width and the channel count, never by the
  number of samples) and the scalars it was built with. It keeps no copy of
  your samples, so any call that rebuilds the envelope needs the source.
- **M4. Recompute.** A full call reduces from scratch. Only an append adds
  samples, and only a sliding append drops columns from the front. An
  append never changes the scale. Only the four calls that take the source
  (`set_monotonic`, `append_monotonic`, `resize`, `fit_layout`) move the
  mapping; no general chart call reaches a monotonic chart, and the
  document editor refuses an edit that would move it.
- **M5. Not streaming.** Monotonic series do not use the streaming runtime,
  and a monotonic series in a streamed display is refused.
- **M6. Interactions.** Every interaction of a line series works (see
  [Interactions](#interactions-m6)). Hover, click selection,
  `step_selection` and the picked highlight report the sample index of a
  kept point and need no source.
- **M7. No count limits.** Any number of samples, channels and columns, and
  any stride, can be used.

## 한국어

단조 x 선 차트는 데이터 수집 기록이나 실시간 센서 값처럼 간격이 일정한 대용량 신호를 화면
해상도에 맞춰 선으로 그린다. 한 묶음의 채널은 모두 같은 x 격자를 쓴다. 샘플 `i`는
`x = start + i × interval`에 놓이며 `interval > 0`이다. 차트는 픽셀 열마다 그 열의 샘플 가운데
첫 샘플, 가장 낮은 샘플, 가장 높은 샘플, 마지막 샘플 넷을 남기고(**M4** 축약) 그 점들을 잇는
선을 그린다. 그림은 모든 샘플을 이은 선과 같아 보이며([정확도](#정확도) 참고), 프레임 비용은
샘플 수가 아니라 플롯 너비와 채널 수로 정해진다.

이 차트는 선만 그린다. 산점도로 그리면 열마다 남긴 네 샘플 사이의 점이 사라지므로 산점도는
제공하지 않는다.

샘플은 호스트 메모리에 그대로 있다. 샘플을 읽는 호출마다 `MonotonicSource`로 빌려주며, 호출이
끝나면 차트는 소스를 참조하지 않는다. 차트가 묶음마다 보관하는 것은 **엔벌로프**, 곧 채널별로
픽셀 열마다 남긴 점이다.

소스 타입과 엔벌로프는 `model::monotonic`에 있고 `figgy-renderer`가 `renderer::monotonic`으로
다시 내보낸다. 차트 호출은 렌더러의 `MonotonicChart`에 있다. 브라우저 제품은 같은 호출을 타입
배열로 제공한다.
[WASM.md의 단조 x 선](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#monotonic-lines)을
참고한다.

### 빠른 시작

위 [Quick start](#quick-start)의 Rust 예제를 참고한다. 채널마다 시리즈를 하나씩 만들어
`register_monotonic_chart`로 등록하고, `set_monotonic`으로 전체 데이터를 넘긴 뒤,
`append_monotonic`으로 새 샘플만 덧붙인다. 확대·이동은 새 x 범위로 하는 전체 호출이고, 크기
조정과 배치 맞춤도 묶음의 소스를 받는다. 편집기, 그리기, 내보내기까지 담은 예제는
[USAGE.md](https://github.com/shim9610/figgy/blob/master/crates/renderer/USAGE.md#단조-x-선)에 있다.

`tools/playground`의 플레이그라운드에 사용 예가 있다. "Monotonic x (recorded channels)" 장면은
채널 넷에 각 200 000개 샘플을 담는다. 장면은 샘플을 직접 들고 있다가 소스를 받는 호출마다
빌려준다.

### 샘플 빌려주기

`MonotonicSource`는 묶음 하나를 나타낸다. 격자(`start()`, 그리고 유한한 양수인 `interval()`),
채널마다 샘플 `len()`개, 채널 `channel_count()`개를 알려 주며, 호출 한 번 동안 빌려준다. 흔한
배치는 내장 소스 두 가지로 처리한다.

| 소스 | 채널 `n`의 샘플 `i` |
|---|---|
| `MonotonicSlices::new(start, interval, &[&[T]])` | `slices[n][i]` |
| `MonotonicInterleaved::new(start, interval, values, stride, columns)` | `values[i * stride + columns[n]]` |

- `MonotonicSlices`는 채널마다 슬라이스를 하나씩 받는다. 슬라이스 길이가 다르면 공통 길이만
  빌려주고, 호출 결과에 `truncated`를 알린다.
- `MonotonicInterleaved`는 프레임마다 값이 `stride`개인 샘플 순서 버퍼 하나를 받으며, 채널 `n`은
  프레임의 `columns[n]`번째 값을 읽는다. 타임스탬프처럼 어느 채널도 지정하지 않은 값은 건너뛴다.
  끝에 덜 채워진 프레임은 다 찬 뒤에 센다. `new`는 `Result`를 돌려주며, `stride`가 0이면
  `InvalidStride`, 프레임 밖의 열을 지정하면 `InvalidColumn`으로 실패한다.
- 원소 형식 `T`는 `f64`, `f32`, `i32`, `i16`, `u16`, `u8` 가운데 하나다. 정수 원시 샘플도 `f64`로
  바꾸지 않고 그대로 빌려줄 수 있다.
- 부동소수 NaN과 무한대는 선의 끊김이다. 정수에는 끊김이 없다.

링 버퍼처럼 다른 배치는 `MonotonicSource`를 직접 구현한다. `lend(range, visit)`는 요청받은 샘플을
연속 구간(`MonotonicRun::slices` 또는 `MonotonicRun::interleaved`)으로 나눠 순서대로, 빠짐도
겹침도 없이 `visit`에 넘긴다. 링 버퍼는 한 바퀴 돈 범위를 두 구간으로 넘긴다.

### 차트

단조 시리즈가 있는 차트는 `Renderer::register_monotonic_chart(config, series)`로 등록한다. 이
호출은 `MonotonicChartId`를 돌려주고, `Renderer::monotonic_chart(id)`가 차트 호출을 담은
`MonotonicChart`를 빌려준다. 차트에 일반 시리즈를 함께 넣어도 된다.

그리는 채널마다 시리즈가 하나씩이며 각자 id, 라벨, 색, 스타일, 범례 항목을 가진다.
`SeriesConfig::monotonic_line(id, group, channel)`이 시리즈를 만든다. 형태는
`DataRenderType::MonotonicLine(MonotonicLineSeries { group, channel, line })`이다. 같은 묶음을
가리키는 시리즈는 그 묶음의 소스를 함께 쓰고, 각 시리즈는 0부터 센 `channel`번 채널을 그린다.
JSON 형태는 `{"MonotonicLine": {"group": .., "channel": .., "line": {..}}}`이다.

문서에는 샘플이 들어가지 않는다. 시리즈의 `x_column`과 `y_column`은 차트가 엔벌로프를 그릴 때
읽는 풀 컬럼 두 개의 이름이며, 도우미는 이를 `<id>.monotonic.x`, `<id>.monotonic.y`로 짓는다.
이 컬럼은 차트만 쓴다. 호스트가 직접 쓰거나 지우면 `ReservedColumnId`로 실패한다.

문서가 축약에 맞지 않으면 차트가 거부한다.

- 선은 직선으로 잇고(`line.interpolation`이 `Linear`) 채우지 않는다. 단조 선에는 마커가 없다.
- x 축은 선형이고 수치형이며 끊김이 없어야 한다. 로그·확률·범주형·끊긴 x 축은 거부한다.
- 차트는 직교 2D 차트여야 한다. 극좌표·삼각·평행좌표·3D 차트는 거부한다.

일반 차트 호출(`set_chart_config`, `set_chart_state`, `set_chart_view_state`,
`set_chart_display`, `hold_chart_display`, 자동 맞춤 호출, 스트리밍 등)은 `ChartId`를 받는다.
단조 차트는 `ChartId`를 내주지 않으므로 이 호출을 단조 차트에 쓰면 컴파일러가 막고,
`chart_order()`에도 단조 차트는 나오지 않는다. 반대로 `register_chart`, `set_chart_series`,
`set_chart_state`, `begin_replace_chart_columns`는 단조 선 시리즈를 거부한다
(`MonotonicGeneralChart`).

### 호출

**x-픽셀 매핑**은 샘플이 어느 픽셀 열에 들어갈지를 정한다. x 범위, 장치 픽셀 단위의 플롯 너비,
x 축 스케일·끊기·방향이 여기에 속한다. 차트는 엔벌로프만 보관하므로 매핑이 바뀌면 샘플을 다시
받아야 한다. 매핑을 바꾸는 호출은 넷이며 모두 소스를 받는다.

| `MonotonicChart::` | 소스 | 하는 일 |
|---|---|---|
| `set_monotonic(group, &source, range)` | 전체 데이터 | 묶음을 처음부터 축약하고 x 범위를 정한다. |
| `append_monotonic(group, &new, MonotonicAppend { sliding })` | 새 샘플만 | 샘플을 덧붙인다. `sliding`이면 창을 민다. |
| `resize(chart_area, plot_area, surface, &[MonotonicGroupRef])` | 옮겨지는 묶음의 소스 | 차트, 플롯, 표시 크기를 바꾼다. |
| `fit_layout(&[MonotonicGroupRef])` | 옮겨지는 묶음의 소스 | 배치를 라벨과 제목에 맞춘다. |

나머지는 소스 없이 동작한다. 문서 편집기(`edit_config`), `set_series`(스타일, 채널, 묶음 변경,
시리즈 추가와 삭제), `step_selection`, `set_selection`, `pick_data`, `pick`, `prepare_picking`,
`remove`, 그리기, 화면과 같은 배율의 내보내기가 그렇다. 엔벌로프는 데이터 단위이므로 y 범위·y
스케일, 색, 선 굵기와 모양, 제목, 눈금, 범례, 주석을 바꿔도 소스가 필요 없다.

#### `set_monotonic`: 전체 호출

`set_monotonic(group, &source, range)`는 `group`을 가리키는 모든 시리즈를 각자 그려지는 패널의
매핑에서 처음부터 축약한다. x 범위를 정하는 호출은 이것 하나뿐이다. `range`는 다음 가운데
하나다.

- `MonotonicRange::X { min, max }`: 이 x 범위를 묶음의 시리즈가 그려지는 x 축에 쓴다. 축의
  나머지 설정과 y 범위는 그대로 둔다. 확대와 이동은 새 범위로 하는 전체 호출이다.
- `MonotonicRange::Autoscale`: x 범위를 샘플이 차지하는 구간
  `start ..= start + (len - 1) × interval`로 정하고, 눈금 계획은 자동 맞춤과 같은 방식으로 맞춘다.
  묶음이 그리는 y 축마다, 그 축에 그려지는 모든 단조 시리즈(이 묶음과 다른 묶음 모두)가 플롯
  안에 남긴 가장 낮은 값과 가장 높은 값에 맞추고, 양쪽에 폭의 5 %씩 여유를 둔다. 그래서 한 축에
  채널이나 묶음이 둘이면 둘 다 화면 안에 들어온다. 기본
  레이어 시리즈는 패널의 `left_y`(짝 축 포함)를, 오버레이 시리즈는 오버레이 자신의 축을(오버레이의
  `link_x`가 꺼져 있으면 자기 x 축도, `link_y`가 켜져 있으면 패널의 `left_y`를),
  서브플롯 시리즈는 서브플롯의 축(x 축은 연결의 맨 처음 축)을 맞춘다. 샘플이 하나뿐이거나 채널
  값이 일정해 폭이 없으면 그 축은 그대로 둔다. 일반 자동 맞춤 호출은 단조 차트에 닿지 않으므로
  단조 차트는 이 방법으로 맞춘다.

이 호출은 화면에 보이는 샘플, 플롯 양쪽 바깥 64픽셀 열에 해당하는 샘플, 그리고 그 양 끝의
샘플 하나씩만 읽는다. 그래서 긴 기록의 작은 부분을 확대해도 보이는 만큼만 비용이 든다. 차트가
알려 주는 샘플 번호(피킹과 선택)는 이 소스의 처음부터 센다.

#### `append_monotonic`: 새 샘플

`append_monotonic(group, &new, MonotonicAppend { sliding })`는 새 샘플만 받으며, 채널마다 개수가
같아야 한다. `new`의 샘플 `j`는 묶음의 샘플 `len + j`가 되어 `start + (len + j) × interval`에
놓인다. `len`은 묶음이 가진 샘플 수다. 새 샘플은 마지막 전체 호출의 격자를 이어 가므로 `new`의
`start`와 `interval`은 읽지 않으며 x 스케일도 바뀌지 않는다. 덧붙이기는 이전 샘플을 읽지 않으므로
전체 호출 사이에 오래된 데이터를 버려도 된다. 소스를 받는 다음 호출은 그때 빌려준 소스로
축약한다.

- `sliding`이 아니면 창은 그대로다. 창 안에 들어오는 새 샘플은 그리고, 창 오른쪽 끝을 넘는
  샘플은 세기만 하고 남기지 않는다. 다만 선이 이어지는 첫 샘플 하나는 남긴다.
- `sliding: true`이면 창 길이를 유지한 채 x 범위를 정확히 `new.len() × interval`만큼 옮기고, 그
  범위를 x 축(패널의 `bottom_x`, `link_x`를 끈 오버레이의 묶음이면 그 오버레이 자기 `x_axis`, 서브플롯 자신의 축이면 연결의 맨 처음 축)에 쓴다. 픽셀 열은
  마지막 전체 호출의 격자에 고정된다. 창 왼쪽으로 벗어난 열은 버리고 새 샘플은 오른쪽에 합친다.
  그래서 열보다 작게 밀면 다음 전체 호출이 다시 맞출 때까지 전체 호출과 픽셀 한 열 미만만큼
  다를 수 있다.
- 세기만 하고 남기지 않은 샘플, 곧 `sliding` 없이 덧붙여 창 오른쪽 끝 너머에 놓인 샘플이 밀어내기로
  화면에 들어와야 하면 `SlidePastKept`로 실패한다. 이때는 소스를 넘겨 전체 호출을 한다.

묶음의 첫 전체 호출 전에 덧붙이면 `NotBuilt`, 채널 수가 다른 소스를 덧붙이면
`ChannelCountChanged`로 실패한다.

#### `resize`와 `fit_layout`

`resize(chart_area, plot_area, surface, &groups)`는 차트 영역, 데이터 영역 크기 조정이 제안한
플롯(데이터 영역, 문서 픽셀 단위. `None`이면 여백 유지), 장치 픽셀 단위의 표시 표면(`None`이면
표시 없음)을 정한다. `fit_layout(&groups)`는 차트 자신의 글꼴로 잰 라벨과 제목에 배치를 맞추며,
이때 플롯 경계가 움직일 수 있다.

`groups`에는 `MonotonicGroupRef::new(group, &source)`로 소스를 담는다. 호출이 차트를 바꾸면,
넘긴 소스가 있는 묶음은 모두 그 소스로 다시 축약한다. 넘기지 않은 묶음은 시리즈가 같은 장치
픽셀에 머물면 엔벌로프를 그대로 쓰고, 옮겨지면 호출이
`FiggyError::MonotonicSourceRequired { series_id, change }`로 실패한다. 모든 묶음의 소스를 넘기면
언제나 안전하다. 아무것도 바꾸지 않는 호출은 소스를 읽지 않는다.

두 호출은 원자적이다. 거부되면 문서, 표시, 그림 가운데 어느 것도 바뀌지 않는다. 여백으로 놓을 수
없는 플롯 사각형은 `NoFeasibleLayout`으로 거부한다.

#### 문서 편집

`edit_config(|editor| ..)`는 문서 사본 위의 `MonotonicConfig`를 빌려준다. `config()`로 `Config`
전체를 읽을 수 있고, 편집기로는 매핑에 영향이 없는 것을 모두 바꿀 수 있다. 범례, 차트 제목, 격자,
주석, 데이터 선택(`picked_data`), 활성 레이어, 배경, 각 축(`MonotonicAxis`: 제목, 눈금 라벨과
형식, 눈금, 간격, 축선, y 축이면 범위), 오버레이 축(`overlay_axis`, 오버레이 자기 x 축은
`overlay_x_axis`이며 그 범위는 단조 호출이 정한다)과 범례, 같은 규칙을 따르는 서브플롯이 여기에
속한다. x 범위, x 스케일, 끊기, 방향, 차트 영역, 플롯 영역을 바꾸는 메서드는 없다.

꾸밈 요소 끌기는 `nudge`와 `drag`로 하며, 플롯 너비를 바꾸는 축이나 플롯 가장자리, 서브플롯 틀처럼
매핑을 바꿀 움직임은 거절한다. `assign(config)`는 데이터로 편집한 문서 전체를 바꾸며, 매핑을
바꾸는 문서는 거부한다. 고친 사본이 결국 매핑을 바꾸면 편집은 `MonotonicMappingEdit`로 실패하고
아무것도 바뀌지 않는다.

#### 읽기, 그리기, 내보내기

- 읽기는 `&Renderer`로 한다. `monotonic_config`, `monotonic_series`, `monotonic_selection`,
  `monotonic_display_config`, `monotonic_render_stamp`, `monotonic_series_draw_info`,
  `monotonic_kept_point`(샘플 번호로 찾은 남긴 점의 `(x, y)`), 그리고 아직 전체 호출을 한 번도
  받지 않은 묶음의 시리즈를 알려 주는 `monotonic_needs_source`(`MappingChange::NotBuilt`)가 있다.
  이 시리즈는 `set_monotonic` 전까지 아무것도 그리지 않는다.
- `Renderer::prepare_monotonic`과 `WindowedRenderer::draw_monotonic`이
  `MonotonicChartDrawItem`을 그린다. 프레임을 그릴 때는 소스가 필요 없다.
- `ExportSource::Monotonic { chart, monotonic: &groups }`로 차트를 PNG, SVG, PDF로 내보낸다.
  화면과 같은 매핑이면 `&[]`를 넘긴다. 2배 출력이나 다른 배율로 표시 중인 차트의 1배 출력처럼
  매핑이 다르면 소스를 넘기며, 묶음마다 그 출력에만 쓰는 엔벌로프를 출력 매핑으로 축약한다.
  소스가 빠진 시리즈가 있으면 `MonotonicSourceRequired`로 실패한다.

축약하는 호출은 모두 `MonotonicUpdate`를 돌려준다. `len`(묶음이 가진 샘플 수), `truncated`,
`dropped`(밀어내기로 버린 열 수), `x_range`(호출 뒤의 x 범위), `pixel_offset`(창이 열 격자보다
얼마나 앞서 밀렸는지를 열의 비율로 나타낸 `0..1` 값. 차트는 이만큼 열을 옮겨 그린다)이 담긴다.

### 비용

- **프레임마다** 채널 하나는 샘플 수와 상관없이 선 파이프라인으로 `8 × (플롯 너비 + 131)`행을
  그린다. 픽셀 열마다 점 넷과 각 점 뒤의 이음 행이며, 플롯과 양쪽 64열 여백을 포함한다. 그중
  절반 정도는 아무것도 그리지 않는다.
- **메모리**: 묶음마다 그 시리즈가 그려지는 매핑별로 CPU에 열 기록 `채널 수 × (플롯 너비 + 131)`개를
  두며, 너비가 다른 서브플롯이 있으면 하나 더 생긴다. 시리즈마다 `8 × (플롯 너비 + 131)`행짜리 풀
  컬럼 두 개를 둔다. 어느 것도 샘플 수에 따라 늘지 않는다. 여기서 플롯 너비는 장치 픽셀 단위다.
- **전체 호출**은 보이는 샘플과 여백의 샘플을 한 번씩 읽는다.
- **덧붙이기**는 새 샘플만 읽는다.

### 정확도

같은 샘플을 보통 선으로 그린 차트와 비교하면 다음과 같다.

- **희소한 보기**(픽셀 열마다 보이는 샘플 4개 이하)는 샘플을 모두 남기므로 1배와 2배에서 그림이
  바이트까지 같다.
- **조밀하거나 섞인 보기**는 열마다 첫 샘플, 가장 낮은 샘플, 가장 높은 샘플, 마지막 샘플로 열의
  범위를 그린다. 한쪽 그림이 칠한 픽셀은 모두 다른 쪽이 칠한 픽셀에서 1픽셀 안에 있으며, 1배와
  2배, 가는 선과 굵은 선에서 모두 그렇다. 바이트까지 같지는 않다. 전체 그리기는 샘플마다
  안티에일리어싱된 선을 하나씩 쌓고, 선 가장자리가 샘플의 열 안 위치에 따라 이웃 열까지 번지기
  때문이다.
- **덧붙이기와 열 단위 밀어내기**는 같은 창을 전체 호출한 결과와 바이트까지 같다. **열보다 작은
  밀어내기**는 전체 그리기와 1픽셀 안에서 같다.
- **픽셀 한 열보다 좁은 끊김**은 이어서 그린다. 화면 해상도로는 구분할 수 없는 끊김이다. 더 넓은
  끊김은 선을 끊으며, 끊김이 한 열을 차지할 때까지 확대하면 보인다.
- 남긴 점은 밀어냈든 아니든 전체 그리기와 정확히 같은 자리에 놓인다.

### 상호작용(M6)

- **피킹**(`pick`, `pick_data`, 마우스 올림, 클릭 선택)은 일반 선과 마찬가지로 포인터 아래 그려진
  선분에서 가까운 쪽 끝의 남긴 점을 골라 그 샘플 번호를 알려 준다. 조밀한 열의 픽셀은 정확히
  그 열의 남긴 점으로 그린다.
- **선택 강조**는 선택한 샘플 번호의 남긴 점에 고리를 그린다. 엔벌로프가 남기지 않은 샘플에는
  고리가 없다.
- **`step_selection`**은 남긴 점을 따라 이동한다.
- **범례**에는 시리즈 색의 선 기호가 나오며, 테마 색 순환이 색을 바꾼다.
- **레이어**: 오버레이와 서브플롯(자기 x 범위와 너비)을 모두 지원한다. 오버레이는 기본으로 부모의
  x 축을 따르고, `link_x`를 끄면 자기 x 축을 가진다. 그래서 다른 시계나 다른 간격으로 기록한
  묶음도 같은 플롯에 겹치고, 묶음마다 전체 호출·추가·Autoscale이 자기 x 축만 움직인다(플레이그라운드의
  "Monotonic x (two clocks)" 장면). 한 x 축을 함께 쓰는 묶음은 함께 움직이므로, 그 축을 움직이는
  호출에는 모든 묶음의 소스가 필요하다.
- **꾸밈 요소와 플롯**: 제목, 범례, 축 제목, 눈금 라벨, 플롯 끌기는 편집기의 `nudge`와 `drag`로
  한다. 데이터 영역 크기 조정은 플롯 사각형을 넘기는 `resize`이므로 소스를 받는다.
- **내보내기**: PNG, SVG, PDF를 어떤 배율로도 내보낸다. 매핑이 화면과 다르면 소스를 함께 넘긴다.
- **스트리밍**: 스트리밍 표시에 든 단조 시리즈는 거부한다(`StreamedMonotonic`).

### 오류

축약에서 난 오류는 `FiggyError::Monotonic { group, error }`에 `MonotonicError`로 담겨 온다.

| `MonotonicError` | 원인 |
|---|---|
| `InvalidGrid { start, interval }` | `start`가 유한하지 않거나, `interval`이 유한한 양수가 아니다. |
| `InvalidMapping { .. }` | x 범위가 유한하지 않거나 증가하지 않는다. 또는 플롯 너비가 0이다. |
| `UnsupportedXMapping` | x 축이 선형이 아니거나 끊겨 있다. |
| `InvalidStride`, `InvalidColumn { column, stride }` | `MonotonicInterleaved::new`에 0인 `stride`나 프레임 밖의 열을 넘겼다. |
| `NotBuilt` | 묶음의 첫 전체 호출 전에 덧붙였다. |
| `ChannelCountChanged { expected, found }` | 채널 수가 다른 소스를 덧붙였다. 전체 호출을 해야 한다. |
| `SlidePastKept` | 세기만 하고 남기지 않은 샘플이 밀어내기로 화면에 들어온다. 전체 호출을 해야 한다. |
| `CountOverflow` | 묶음의 샘플 수가 `usize`를 넘친다. |
| `OutOfMemory` | 엔벌로프 메모리를 할당하지 못했다. |
| `SourceRequired { change }` | `MonotonicEnvelope::check_mapping`이 다른 매핑을 발견했다(코어 API 전용). |

차트가 직접 거부하는 경우는 다음과 같다. `RuleKind`로 적은 오류는 오류의 `kind` 필드에 그 값이
들어 있다.

| 오류 | 원인 |
|---|---|
| `FiggyError::UnknownMonotonicGroup { group }` | 그 묶음을 가리키는 시리즈가 차트에 없다. |
| `FiggyError::MonotonicSourceRequired { series_id, change }` | 크기 조정, 배치 맞춤, 내보내기가 소스를 넘기지 않은 묶음의 시리즈를 옮긴다. 아무것도 바뀌지 않는다. |
| `FiggyError::ReservedColumnId { id }` | 호스트가 엔벌로프 컬럼을 쓰거나 지우려 했다. |
| `RuleKind::MonotonicRange` | `MonotonicRange::X`의 범위가 유한하지 않거나 증가하지 않는다. |
| `RuleKind::MonotonicMappingEdit` | 편집기에서 매핑을 바꾸려 했다. |
| `RuleKind::NoFeasibleLayout` | 크기 조정의 플롯 사각형을 여백으로 놓을 수 없다. |
| `RuleKind::MonotonicChannel { channel, channels }` | 시리즈가 묶음의 소스에 없는 채널을 가리킨다. |
| `RuleKind::MonotonicColumnTaken { column }` | 엔벌로프 컬럼 id를 이미 호스트 컬럼, 다른 시리즈, 다른 차트가 쓰고 있다. |
| `RuleKind::MonotonicGeneralChart` | 일반 차트 호출에 단조 시리즈를 넘겼다. |
| `RuleKind::StreamedMonotonic` | 스트리밍 표시에 단조 시리즈가 들어 있다. |
| `MonotonicXAxis`, `MonotonicChartKind`, `PolarMonotonic`, `MonotonicInterpolation`, `MonotonicFill`, `MonotonicGroupEmpty`, `MonotonicColumns`, `MonotonicColumnsShared` | 문서가 [차트](#차트)의 규칙(x 축, 차트 종류, 선, 빈 묶음 이름, 엔벌로프 컬럼 id)을 어긴다. |

### 한계

- **선 굵기.** 엔벌로프는 그리는 배율에서 플롯 양쪽 바깥으로 64픽셀 열까지 닿는다. 장치 픽셀로 약
  120픽셀(선 굵기에 표시 배율이나 내보내기 배율을 곱한 값)까지의 선은 가장자리를 전체 그리기와
  똑같이 그린다. 더 굵은 선은 플롯 가장자리의 잉크를 놓칠 수 있다. 굵기 변경은 일반 스타일
  수정이므로 소스가 필요 없다.
- **x 축.** 직교 2D 차트의 선형·수치형·끊김 없는 축만 쓴다.
- **선만 그린다.** 산점도, 마커, 채우기, 계단 보간은 없다.
- **소스 없이 매핑을 바꾸는 몸짓은 없다.** 브라우저에서는 단조 차트의 휠·끌기 확대가 꺼져 있고,
  크기 조정과 데이터 영역 크기 조정은 호스트가 소스를 넘길 때까지 기다린다.
- **스트리밍은 지원하지 않는다.**
- **개수 제한은 없다**(M7).

### 코어 직접 쓰기

`MonotonicEnvelope`는 축약만 따로 떼어 낸 타입이다. 엔벌로프를 직접 그리는 호스트가 렌더러 없이
쓸 수 있다.

- `MonotonicEnvelope::empty()`로 만든다. `set_monotonic(&source, &mapping)`이 전체 호출이고
  `append_monotonic(&new, MonotonicAppend { sliding })`이 덧붙이기이며, 동작은 위와 같다. 둘 다
  `MonotonicUpdate`를 돌려준다.
- `MonotonicMapping`은 x 범위, 장치 픽셀 단위의 플롯 너비, x 축 스케일·끊기·뒤집기, 그리고 플롯
  양쪽 바깥에 둘 픽셀 열 수인 `margin_px`를 담는다.
  `MonotonicMapping::linear(x_min, x_max, plot_width_px)`는 여백이 없는 기본 매핑을 만든다. 샘플
  `i`는 플롯 열 `floor((start + i × interval - x_min) / x_per_pixel)`에 들어가며,
  `x_per_pixel = (x_max - x_min) / plot_width_px`다.
- `check_mapping(&current)`는 `current`가 엔벌로프를 만든 매핑(밀어내기 반영)과 다르면
  `SourceRequired { change }`로 실패한다. 엔벌로프는 스스로 다시 만들 수 없으므로 전체 호출을 한다.
- `is_grid_aligned()`는 열이 현재 x 범위에 맞춰져 있는지(픽셀 오프셋이 없는지) 알려 준다.
  `aligned_mapping()`은 열이 맞춰진 x 범위다.
- 채널마다 슬롯이 `slots()`개, 곧 `w + 3`개 있다. `w`는 플롯 너비에 양쪽 여백을 더한 값이다.
  슬롯은 왼쪽 경계 샘플(열 0 왼쪽의 마지막 샘플), 열 `0..w`, 열 `w`(열보다 작은 밀어내기에서 보일
  수 있으므로 통째로 둔다), 오른쪽 경계 샘플(열 `w` 오른쪽의 첫 샘플) 순서다.
  `cell(channel, slot)`은 점을 최대 4개 담은 `EnvelopeCell`을 돌려주며, 점마다 값과 샘플 번호가
  있다.
- 채널을 그리려면 셀 안에서 `cell.linked(k)`가 참인 곳의 점 `k`와 `k + 1`을 잇고, 한 셀의 마지막
  점과 다음 비어 있지 않은 셀의 첫 점은 `EnvelopeCell::joins`가 참일 때(둘 다 유한하고 샘플
  번호가 연속) 잇는다.
- `column_of_x(x)`는 x가 놓인 열을, `pick(channel, column, y)`는 그 열에 남긴 점 가운데 `y`에 가장
  가까운 점의 샘플 번호와 값을 돌려준다.

### 보장(M1–M7)

단조 선 시리즈가 지키는 약속이다. 다른 문서는 이를 M1–M7로 가리킨다.

- **M1. 입력.** 샘플은 값과 `start`, `interval`을 함께 담은 `MonotonicSource`로만 들어오며,
  샘플을 읽는 호출마다 넘긴다. `start`나 `interval`을 따로 받는 호출은 없다. 샘플을 컬럼으로
  올리지 않는다.
- **M2. 축약.** 픽셀 열마다 첫 샘플, 가장 낮은 샘플, 가장 높은 샘플, 마지막 샘플로 줄인다. 전체
  호출 뒤나 열 단위 변경 뒤에는 열마다 모든 선분을 그릴 때와 같은 범위를 덮으며, 희소한 보기는
  전체 그리기와 같은 그림을 그린다. 열보다 작게 밀어내는 동안에는 다음 전체 호출이 다시 맞출
  때까지 픽셀 한 열 미만만큼 다를 수 있다. 열보다 좁은 끊김은 보이지 않는다.
- **M3. 메모리.** 차트는 엔벌로프(열마다 점 4개 이하. 크기는 플롯 너비와 채널 수로 정해지며 샘플
  수와 무관하다)와 그것을 만들 때 쓴 스칼라 값만 보관한다. 샘플 복사본을 두지 않으므로
  엔벌로프를 다시 만드는 호출은 모두 소스를 받는다.
- **M4. 재계산.** 전체 호출은 처음부터 축약한다. 샘플을 덧붙이는 것은 `append`뿐이고, 앞쪽 열을
  버리는 것은 밀어내기 `append`뿐이다. 덧붙이기는 스케일을 바꾸지 않는다. 매핑은 소스를 받는 호출
  넷(`set_monotonic`, `append_monotonic`, `resize`, `fit_layout`)만 옮긴다. 일반 차트 호출은 단조
  차트에 닿지 못하고, 문서 편집기는 매핑을 옮길 편집을 거부한다.
- **M5. 스트리밍과 무관.** 단조 시리즈는 스트리밍 런타임을 쓰지 않으며, 스트리밍 표시에 든 단조
  시리즈는 거부한다.
- **M6. 상호작용.** 선 시리즈의 상호작용을 모두 지원한다([상호작용](#상호작용m6) 참고). 마우스
  올림, 클릭 선택, `step_selection`, 선택 강조는 남긴 점의 샘플 번호를 알려 주며 소스가 필요 없다.
- **M7. 개수 제한 없음.** 샘플, 채널, 열 수와 간격(stride)에 상한이 없다.
