# Monotonic-x line core / 단조 x 선 코어

The CPU core of the monotonic-x line chart (#271), in
`model::monotonic`. It serves fast, large, evenly spaced data:
`x = start + i * interval`, strictly increasing. Per pixel column it reduces
the samples to **M4**: the first and last samples, and the lowest and
highest. A renderer then draws about `4 × plot width` vertices per channel
however many samples there are. It draws lines only: scatter is out of
scope, because keeping the extremes per column drops the points between
them.

The core is pure CPU and lives here. The renderer keeps one envelope per
group, uploads it and draws it with the line pipeline, and the browser
product passes typed arrays: see [In the renderer](#in-the-renderer).

## Source

The host lends its samples through the `MonotonicSource` trait, for one call
at a time; the envelope keeps no reference to it. A source carries its x
grid, `start()` and `interval() > 0` (sample `i` is at `start + i *
interval`), has `len()` samples per channel and `channel_count()` channels,
and lends a requested
range as contiguous runs (`MonotonicRun`), in order, through a callback,
without allocating. A run is either one slice per channel or frames of an
interleaved buffer, both read in place, so the reduction gets contiguous
memory to vectorize. A ring buffer lends a wrapped range as two runs.

| Built-in impl | Channel `n`, sample `i` |
|---|---|
| `MonotonicSlices::new(start, interval, &[&[T]])` | `slices[n][i]` |
| `MonotonicInterleaved::new(start, interval, values, stride, columns)` | `values[i * stride + columns[n]]` |

In an interleaved buffer a column no channel names (a timestamp, say) is
never read, and a trailing partial frame is not counted until it is
complete. When per-channel slices differ in length, the source lends their
common extent and reports `truncated`.

The element type is one of `f64`, `f32`, `i32`, `i16`, `u16` and `u8`
(`MonotonicElement`, sealed), and every type goes through the same generic
path. The reduction compares in the element type and converts only the kept
values to `f64`; nothing is converted or copied in bulk. A non-finite float
(NaN or ±infinity) is a gap in the line. Integers have no gaps.

The full call reads the grid; an append continues the full call's grid and
reads neither `start` nor `interval` (no call takes them separately).

## Columns

The mapping (`MonotonicMapping`: x range, plot width in device pixels, x
scale, x breaks, x inversion, margin) places sample `i` at
`t = (start + i * interval - x_min) / x_per_pixel`, with
`x_per_pixel = (x_max - x_min) / width`. Plot column `c` holds the samples
with `c <= t < c + 1`. The envelope keeps `margin_px` whole columns beyond
each edge of the plot as well, reduced like the plot's own: a stroke
reaches into the plot from samples up to its half width outside it, so a
renderer keeps a margin its strokes reach from (the renderer's is a fixed
64 columns). Envelope column `k` is
plot column `k - margin`. Every boundary is plain arithmetic on these scalars,
corrected by a step or two where rounding needs it, never a search over the
data (`MonotonicGrid`). A logarithmic, probability or broken x axis is not
meaningful for this source and is refused with
`MonotonicError::UnsupportedXMapping`.

Per channel the envelope keeps `w + 3` slots, where `w` is the plot width
and twice the margin:

- the lead edge sample, the last sample left of envelope column 0;
- envelope columns `0..w`;
- envelope column `w`, kept in full so that a slide by part of a column can
  show it;
- the trail edge sample, the first sample right of envelope column `w`.

## The envelope

`MonotonicEnvelope` is the CPU cache of one group: one buffer of
`channels × (width + 3)` column accumulators, channel-major. Its size
depends on the plot width and the channel count, never on the sample
count. `cell(channel, slot)` gives a slot's drawn form, an `EnvelopeCell` of
up to 4 points. Each point carries its value in data units and its logical
sample index, so a pick maps back to the host's data.

- A column with at most 4 samples keeps them all.
- A denser column keeps its first and last samples and the lowest and
  highest *drawn* samples (the first lowest and the last highest). A drawn
  sample is a finite sample with a finite neighbour in the same column, that
  is, an end of a segment inside the column. The first and last samples
  join the neighbouring columns, so a column is reduced from its own samples
  alone.
- The first or last sample joins an extreme only inside one run of finite
  samples. The two extremes are always joined, so the column covers the span
  its drawn samples cover. A gap narrower than one column that separates two
  runs inside that column is therefore drawn joined: it is not resolvable at
  display resolution, so this is the correct image (the maintainer's
  decision, #271). A gap at least one column wide breaks the line, and
  zooming in until a gap spans a column shows it.

The polyline the envelope implies works as follows. Inside a cell, point
`k` joins point `k + 1` where bit `k` of `links` is set. The last point of
one cell joins the first point of the next non-empty cell when both are
finite and their indices are consecutive (`EnvelopeCell::joins`). In every
kept column this polyline touches the same vertical span as drawing every
segment, and it keeps the column's entry and exit samples. The unit tests
check this against drawing every sample.

The accumulator is closed under appending. A column fed its samples in any
number of pieces equals the same column fed them at once: the first sample
stays, the extremes update, and the last sample is replaced. A sparse column
holds all of its samples, so it merges exactly too. This is what lets an
append work without the old samples.

The envelope is in data units, so a change of the y range or y scale,
colours, line style, titles, ticks, legend or annotations needs no new
reduction, as long as the plot width does not change.

## Calls

| Call | Takes | Reads |
|---|---|---|
| `set_monotonic(&source, &mapping)` | the whole data, with its `start` and `interval` | the lead edge, columns `0..=width` and the trail edge, nothing else |
| `append_monotonic(&new, MonotonicAppend { sliding: false })` | only the new samples | only the new samples, up to the trail edge |
| `append_monotonic(&new, MonotonicAppend { sliding: true })` | only the new samples | only the new samples, up to the trail edge |

- **Full call.** Registering or setting data, and any change of the mapping,
  is a full call with the source. It reduces from scratch every time.
- **Append.** Sample `j` of `new` is logical sample `len + j`, at
  `start + (len + j) * interval`, where `len` is the envelope's own count.
  The call compares nothing and never changes the scale. New samples in the
  kept columns are merged; later ones are counted, not kept, except the
  trail edge sample. No call but the full one reads earlier samples, so the
  host is free to drop old data. A source with another channel count is
  refused.
- **Sliding append.** The new samples are appended, and the x range moves by
  exactly `new.len() × interval`. The columns stay anchored to the grid the
  full call built: they are not re-binned. The columns that fully leave the
  window on the left are dropped. The last kept point of the columns that
  left becomes the lead edge sample, and the new samples merge at the back
  on the same grid. The fraction of a column the range has moved past the
  grid is reported as `pixel_offset` (in `0..1`), so phase 2 offsets the
  projection by it. A slide never reads old samples and never falls back to
  a full reduction. Old samples that were counted but not kept, more than one
  past the kept columns, would have to come into view; then the slide is
  refused with `MonotonicError::SlidePastKept`.
- **Result.** Every call returns a `MonotonicUpdate`: the logical length,
  `truncated`, the dropped columns, the x range for the host's x axis, and
  the pixel offset.
- **Alignment.** `is_grid_aligned()` says whether the columns are aligned to
  the current x range: no pixel offset, and a full call on the current
  mapping would put every sample in the same column. It checks the column
  boundaries by arithmetic, without the data. `aligned_mapping()` is the x
  range the columns are aligned to.
- **Mapping record.** The envelope records the mapping it is drawn for, and
  a slide moves that mapping's x range. `check_mapping(current)` refuses any
  other mapping with `MonotonicError::SourceRequired { change }`. That covers
  the x range, the plot width in device pixels (a resize, a layout change
  that moves the plot edges, a DPI change, a 2x export), the x scale, x
  breaks and x inversion. The envelope keeps no copy of the data, so it
  cannot rebuild itself; only a full call with the source does.
- **Picks.** `column_of_x(x)` gives the column under an x, and
  `pick(channel, column, y)` the kept point nearest to `y`, as its logical
  sample index and value. Neither needs the source. A dense column's drawn
  pixels come from exactly these points.

The reduction runs on the CPU in chunked loops that the compiler
vectorizes, without a SIMD crate. A contiguous run is scanned once, in
blocks with independent lane accumulators; only the block that holds the
lowest (or highest) value is scanned again for its position. An interleaved
run is walked frame by frame, a batch of channels at a time, so each
frame's cache line is read once per batch. The batch is a loop width, not a
limit.

## In the renderer

The renderer side (`figgy-renderer`, `renderer::monotonic.rs`) keeps every
rule above and adds the document, the monotonic chart handle, the GPU draw
and the interactions.

### The document

`DataRenderType::MonotonicLine(MonotonicLineSeries { group, channel, line })`
is one channel of a group, drawn as a line: each channel is its own series,
with its own id, label, colour, style and legend entry. The source is never
in the document. The series' `x_column` and `y_column` are the two pool
columns the renderer keeps the drawn envelope in; the helper
`SeriesConfig::monotonic_line(id, group, channel)` names them
`<id>.monotonic.x` and `<id>.monotonic.y`. They belong to that series of
that chart: a host cannot write or remove them (`ReservedColumnId`), and a
series whose column ids another series or chart owns, or a host column
takes, is refused (`MonotonicColumnTaken`). The line is linear and unfilled.
The chart's x axis must be linear, numeric and unbroken, on a rectangular 2D
chart: a logarithmic, probability, categorical or broken x axis, and polar,
ternary, parallel and 3D charts, are refused with the document
(`Config::validate_monotonic_series`). The wire form is
`{"MonotonicLine": {"group": .., "channel": .., "line": {..}}}`, a new
variant, so older documents read and write as before.

### The monotonic chart

A chart with monotonic series is its own kind of registered chart:
`Renderer::register_monotonic_chart(config, series)` returns a
`MonotonicChartId`, and only that handle reaches it. The general chart
calls (`set_chart_config`, `set_chart_state`, `set_chart_view_state`,
`set_chart_display`, `hold_chart_display`, the auto-fit family, streaming
and the rest) take a `ChartId`, which a monotonic chart never hands out, and
monotonic charts are kept out of `chart_order`: no general call can reach
one, so none can move its x-to-pixel mapping. That is a type-level
guarantee, not a refusal. A monotonic line series lives only in a monotonic
chart: `register_chart`, `set_chart_series`, `set_chart_state` and
`begin_replace_chart_columns` refuse it at registration
(`RuleKind::MonotonicGeneralChart`). Internally the handle wraps the
registry's own chart id, so a monotonic chart draws, picks and exports
through the same paths as any chart.

`Renderer::monotonic_chart(id)` lends a `MonotonicChart`, the chart's calls.
Four move the mapping, and each takes the source:

| `MonotonicChart::` | Does |
|---|---|
| `set_monotonic(group, &source, range)` | The full call for every series of the chart that names `group`, each at the mapping its panel is drawn at. `range` is `MonotonicRange::X { min, max }`, written to the x axis the group's series are drawn against (the rest of the axis stays), or `MonotonicRange::Autoscale` (below). The only call that sets the x range. |
| `append_monotonic(group, &new, MonotonicAppend { sliding })` | Merges only the new samples. A slide writes the moved x range to the document: the parent's `bottom_x`, or a subplot's own axis (the root of its link chain). |
| `resize(chart_area, plot_area, surface, &[MonotonicGroupRef])` | The resize: the chart area, the plot (data area) a plot-area resize proposes (`None` keeps the margins), and the display surface (`None`: no display). Each group whose series it moves to other device pixels is reduced again from its source in the same call. |
| `fit_layout(&[MonotonicGroupRef])` | Fits the layout to the labels and titles (`layout::auto_layout`, measured with the chart's own font) and applies it. It can move the plot's edges, so each group whose series it moves is reduced again from its source. |

The other calls need no source: `edit_config` (below), `set_series` (a
style, a channel, a group, series added or removed), `step_selection`,
`set_selection`, `pick_data`, `pick`, `prepare_picking` and `remove`. Reads
take `&Renderer`: `monotonic_config`, `monotonic_series`,
`monotonic_needs_source` (the series whose group was never supplied, which
draw nothing until `set_monotonic`, `MappingChange::NotBuilt`),
`monotonic_kept_point` (a kept point's `(x, y)`, read from the envelope),
`monotonic_selection`, `monotonic_display_config`, `monotonic_render_stamp`
and `monotonic_series_draw_info`. `Renderer::prepare_monotonic` (and
`WindowedRenderer::draw_monotonic`) draws `MonotonicChartDrawItem`s, and
`ExportSource::Monotonic { chart, monotonic: &[MonotonicGroupRef] }`
exports one, `&[]` at the screen's mapping.

**The document editor.** `MonotonicChart::edit_config(|editor| ..)` lends a
`model::monotonic::MonotonicConfig` over a copy of the document: read access
to the whole `Config` (`config()`) and editors for everything else: the
legend, the chart title, the grid, annotations, the data selection
(`picked_data`), the active layer, the backgrounds, each axis
(`MonotonicAxis`: title, tick labels and their format, ticks, spacing, line;
a y axis also its range), overlay y axes and legends, and subplots under the
same rules. It has no setter for the x range, x scale, breaks, direction or
the chart or plot area, and no `&mut Config`. Decoration drags go through
`nudge` and `drag`, which reject a move that would change the mapping (an
axis or plot edge that changes the plot's width, a subplot frame);
`assign(config)` replaces the whole document (a document edited as data)
and refuses one that moves it. The renderer checks the edited copy with
`same_mapping` (the document size, and per panel the plot width and the x
range, scale, breaks, direction and categories) before it keeps it; an edit
that fails is refused with `RuleKind::MonotonicMappingEdit` and changes
nothing.

**Every call that changes the chart goes through one shared step.** It
places the document the call leaves as the chart's display will show it
and plans each monotonic series: a series whose mapping did not move keeps
its group's envelope, a group whose source the call lends is reduced again
at every mapping its series are drawn at, and a resize or layout fit that
moves a group without its source is refused with
`FiggyError::MonotonicSourceRequired { series_id, change }` before anything
is written: the document, the display, the envelopes and the image stay as
they were. The resize plans the display surface and the document together,
so it is atomic: a refusal changes nothing, and a later failure puts the
display back. A plot rect the margins cannot place is refused with
`RuleKind::NoFeasibleLayout`. The margin beyond the plot's edges is a fixed
64 pixel columns at the scale the envelope is drawn at (`MARGIN_PX`), so a
stroke width is a plain style edit. Frames and picks never fail for a
missing source: a chart's envelopes always match the mapping it is drawn at.
An export at another scale takes the sources and reduces each group at the
export's mapping into transient columns that live for the export alone.

**Auto-scale is a range of the full call.** `set_monotonic(group, &source,
MonotonicRange::Autoscale)` fits the x range of the axes the group's series
are drawn against to `start ..= start + (len - 1) × interval`, by arithmetic
on the source's grid (with the tick plan, as an auto-fit fits an axis), and
their y ranges, padded by 5 % of the span on each side, to the lowest and
highest value each series' envelope keeps in the plot. Those values come
from the reduction the same call makes, which keeps every column's
extremes, so no other pass reads the data (a display parameter, rule 1). A
base series fits the panel's `left_y` (with its partner), an overlay series
its overlay's own axis, a subplot series its subplot's axes (x through the
root of its link chain). An extent without width (one sample, a constant
channel) leaves its axis as it is. The general auto-fit takes a `ChartId`,
so it cannot reach a monotonic chart.

### The draw

Each series' envelope is two pool columns, uploaded through the renderer's
column upsert (`ColumnSource` writing straight into mapped staging), and
drawn by the existing line pipeline (`line_columnar.wgsl`, the `common/`
stroke fragments): no new shader, entry point or pipeline. A slot takes 8
rows: each of its 4 points followed by a join marker that repeats the point
where the line goes on and is NaN where it breaks. A repeated point is a
zero-length segment the stroke collapses; a NaN row masks the segments
that touch it, as in any line. So a row maps to its slot and point by
arithmetic (`row / 8`, `(row % 8) / 2`), which is how a pick's row maps
back to a logical sample index and a selection's index to a row. x is
`start + index × interval` in data units, so the projection places every
kept point exactly where a full draw places it, slide offset or not. Per
frame a channel draws `8 × (w + 3)` rows whatever the sample count; about
half are zero-length or masked. The envelope keeps a fixed margin of 64
whole pixel columns beyond each edge at the scale it is drawn at
(`MARGIN_PX`), the columns a stroke reaches into the plot from. **The
limit:** strokes up to about 120 device pixels wide (a width times the
display or export scale) draw their edges as a full draw does; a wider one
may miss ink at the plot's edges. The margin is a display parameter (rule
1), not a count of data, and no style edit changes it, so a width change
needs no source. A monotonic line has no markers.
(`strokes_up_to_the_margin_limit_draw_their_edges_exactly`: 40 and 60 px
strokes at 1x and 2x, up to 120 device pixels, are the full draw's byte for
byte after a zoom.)

### Pixels (M2 at display resolution)

`tests/monotonic_render.rs` compares a chart drawing a monotonic series with
a chart drawing the same samples as an ordinary line:

- **Sparse views** (at most 4 visible samples per pixel column) keep every
  sample: the images are byte-identical at 1x and 2x.
- **Dense and mixed views** draw each column's span with the column's first,
  lowest, highest and last samples. The images are not byte-identical,
  because the full draw stacks one antialiased stroke per sample and each
  stroke's fringe reaches the neighbouring column from wherever its sample
  lies in its column; the envelope draws four strokes. Every pixel either
  draw paints lies within one pixel of a pixel the other paints, at 1x and
  2x, for hairlines and thick lines.
- **Appends and whole-column slides** draw byte for byte as a full call on
  the same window; **slides by part of a column** stay within one pixel of
  the full draw.
- **A NaN gap narrower than a column** is drawn joined; a wider one breaks
  the line in both draws.
- **Wide strokes** keep their edges up to the limit: 40 and 60 px strokes
  zoomed into a sparse window are byte-identical to the full draw at 1x and
  2x (up to 120 device pixels); a wider stroke may miss edge ink.

### Interactions (M6)

- **Picks** run on the GPU over the drawn envelope, as any line's do, and
  report the logical sample index of the kept point: the ticket maps the
  row back through the envelope it was submitted with. A dense column's
  drawn pixels come from exactly those points.
- **The picked highlight** rings the kept point with the selected logical
  index; a sample the envelope does not keep has no ring.
- **`step_selection`** walks the kept points (`MonotonicChart::step_selection`).
- **Legend**: a line swatch in the series' colour; the theme cycle
  recolours it.
- **Layers**: overlays (the parent's x axis, the overlay's y axis) and
  subplots (their own x range and width, one envelope per mapping a group
  is drawn at).
- **Decorations and the plot**: title, legend, axis-title and tick-label
  drags and a plot-area drag go through the editor's `nudge` and `drag`; a
  plot-area resize is a resize (`MonotonicChart::resize` with a plot rect),
  which takes the sources.
- **Exports**: PNG, SVG and PDF at any scale, with the sources when the
  mapping differs.
- **The scene catalogue**: `SceneKind::Monotonic` in
  `tests/support/playground_scenes.rs` (four channels of 200 000 samples)
  runs in the decoration oracle, the vector export sweeps and the
  `auto_layout` sweep. The scene keeps its samples
  (`CartesianScene::monotonic`) and the sweeps export it through
  `export_scene`, which registers the document as a monotonic chart for
  the call and hands the samples to `ExportSource::Monotonic`. The
  playground registers a monotonic scene as a monotonic chart, follows its
  viewport with `MonotonicChart::resize` and the scene's samples, gives a
  group never supplied its first full call, edits its document through the
  editor, and neither pans, zooms nor holds it.
- **Not streaming** (M5): a monotonic series in a streamed display is
  refused (`StreamedMonotonic`).

The browser product mirrors the handle: `FiggyChart.set_monotonic_document`
shows a monotonic chart, whose setters edit its document through the editor,
and its monotonic calls take typed arrays of all six element types, one per
channel or one interleaved array with a stride (`set_monotonic`,
`append_monotonic`, `resize_monotonic`, `fit_layout`, `needs_source`,
`monotonic_groups`, `export_png_monotonic` and the SVG and PDF forms); its
wheel and drag zoom are off, and a plot-area resize gesture proposes a rect
the host applies; see `WASM.md`.

## Rules for this source (M1–M7)

A separate rule set for monotonic line series only. Rules 1, 2, 5 and 8 of
the repository's `CLAUDE.md` keep governing every other series.

- **M1. Input.** Data enters only through `MonotonicSource`, which carries
  the values, `start` and `interval`, passed to every call that reads it. No
  call takes `start` or `interval` separately. No other container, and no
  `ColumnSource` upload.
- **M2. Reduction is appearance.** The renderer may reduce to M4 per pixel
  column. M2 is stated at display resolution: after a full call or a
  whole-column change, each column covers the span drawing every segment
  covers (a sparse view keeps every sample and draws the same image); while
  sliding by fractions of a column, it differs by less than one pixel
  column, through the anchored grid and its offset, and the next full call
  realigns it exactly; a gap narrower than a column is not resolved.
- **M3. Cache bound.** The CPU cache holds only the column accumulators
  (4 kept samples per column, sized by plot width and channel count, never
  by data length) and the scalars they were built with. Never a copy of the
  data.
- **M4. Recompute.** The full call reduces from scratch. Only `append`
  merges new samples, and only a sliding `append` drops columns from the
  front. The scale never changes inside an append. Only the four calls of
  the monotonic chart handle that take the source move the mapping
  (`set_monotonic`, `append_monotonic`, `resize`, `fit_layout`); no general
  chart call can reach a monotonic chart, and its document editor refuses
  an edit that would move it.
- **M5. Not streaming.** This source does not use, extend or depend on the
  streaming runtime, which stays as it is.
- **M6. Interactions.** Rule 6 applies in full. Hover, click selection,
  `step_selection` and the picked highlight report the logical sample index
  of a kept point, without the source: a pick the kept point at the nearer
  end of the drawn segment under the pointer, as a line's pick does.
- **M7. No count limits.** Rule 10 applies: no cap on samples, channels,
  columns or stride.

## 한국어

`model::monotonic`은 단조 x 선 차트(#271)의 CPU 코어다. 간격이 일정하고
(`x = start + i * interval`) 빠르게 늘어나는 대용량 데이터를 다룬다. 픽셀 열마다 샘플을
**M4**로 줄인다. 첫 샘플과 마지막 샘플, 가장 낮은 값과 가장 높은 값만 남긴다. 그래서
샘플이 몇 개든 렌더러는 채널마다 정점을 `4 × 플롯 너비` 개 정도만 그린다. 선만 지원한다.
산점도는 열마다 극값만 남기면 그 사이 점이 사라지므로 대상이 아니다.

코어는 CPU에서만 돌며 이 크레이트에 있다. 렌더러는 묶음마다 엔벌로프를 하나씩 두고
업로드해 선 파이프라인으로 그린다. 브라우저 제품은 타입 배열을 넘긴다.
[렌더러에서](#렌더러에서)를 참고한다.

### 데이터 소스

호스트는 `MonotonicSource` 트레이트로 샘플을 호출마다 빌려준다. 엔벌로프는 호출이 끝나면
소스 참조를 남기지 않는다. 소스는 x 격자 `start()`와 `interval() > 0`을 함께 담고(샘플
`i`는 `start + i * interval`에 놓인다), 채널마다 `len()`개 샘플과 `channel_count()`개 채널을
가지며, 요청받은 범위를 연속 구간(`MonotonicRun`)으로 나눠 순서대로 콜백에 넘긴다.
메모리를 따로 할당하지 않는다. 구간은 채널별 슬라이스이거나 섞인 버퍼의 프레임이며, 둘 다
제자리에서 읽으므로 축약이 연속 메모리를 벡터화할 수 있다. 링 버퍼는 한 바퀴 돈 범위를 두
구간으로 넘긴다.

- 채널별 슬라이스: `MonotonicSlices::new(start, interval, &[&[T]])`. 채널 `n`의 샘플 `i`는 `slices[n][i]`다.
- 샘플 순서로 섞인 버퍼: `MonotonicInterleaved::new(start, interval, values, stride, columns)`. 채널 `n`의
  샘플 `i`는 `values[i * stride + columns[n]]`이다.

섞인 버퍼에서 어느 채널도 지정하지 않은 열(타임스탬프 등)은 읽지 않으며, 끝에 덜 채워진
프레임은 다 찰 때까지 세지 않는다. 채널별 슬라이스의 길이가 다르면 공통 길이만 빌려주고
`truncated`를 알린다.

원소 형식은 `f64`, `f32`, `i32`, `i16`, `u16`, `u8` 가운데 하나이며(`MonotonicElement`),
모든 형식이 같은 제네릭 코드를 지난다. 비교는 원래 형식으로 하고 남기는 값만 `f64`로
바꾼다. 데이터를 통째로 변환하거나 복사하지 않는다. 유한하지 않은 부동소수(NaN, ±무한대)는
선의 끊김이며, 정수에는 끊김이 없다.

전체 호출이 이 격자를 읽는다. 추가는 전체 호출의 격자를 이어 가므로 `start`도 `interval`도
읽지 않는다. 어떤 호출도 둘을 따로 받지 않는다.

### 열

매핑(`MonotonicMapping`: x 범위, 장치 픽셀 단위 플롯 너비, x 축 스케일, 축 끊기, 뒤집기,
여백)은 샘플 `i`를 `t = (start + i * interval - x_min) / x_per_pixel` 위치에 놓는다.
여기서 `x_per_pixel = (x_max - x_min) / width`다. `c <= t < c + 1`인 샘플이 플롯 열
`c`에 속한다. 엔벌로프는 플롯 양쪽 바깥으로 `margin_px`개 열을 더 두고 플롯 안의 열과
똑같이 축약한다. 선은 굵기의 절반만큼 플롯 바깥에 있는 샘플에서도 플롯 안으로 들어오므로,
렌더러는 선이 닿는 여백을 남겨 둔다(렌더러의 여백은 64열로 고정이다). 엔벌로프 열 `k`는 플롯 열 `k - margin`이다. 열
경계는 모두 이 값들로 계산하고 반올림 오차는 한두 단계 보정으로 맞추며, 데이터를 탐색하지
않는다(`MonotonicGrid`). 로그·확률·끊긴 x 축은 이 소스에 의미가 없으므로
`MonotonicError::UnsupportedXMapping`으로 거부한다.

엔벌로프는 채널마다 슬롯 `w + 3`개를 둔다. `w`는 플롯 너비에 양쪽 여백을 더한 열 수다.

- 왼쪽 경계 샘플: 엔벌로프 열 0 왼쪽의 마지막 샘플
- 엔벌로프 열 `0..w`
- 엔벌로프 열 `w`: 열보다 작은 밀어내기에서 보일 수 있으므로 통째로 둔다.
- 오른쪽 경계 샘플: 엔벌로프 열 `w` 오른쪽의 첫 샘플

### 엔벌로프

`MonotonicEnvelope`는 묶음 하나의 CPU 캐시다. 열 누산기 `채널 수 × (width + 3)`개를 버퍼
하나에 채널 순서로 담으며, 크기는 플롯 너비와 채널 수로만 정해지고 샘플 수와 무관하다.
`cell(channel, slot)`은 슬롯을 그리는 형태, 곧 점을 최대 4개 담은 `EnvelopeCell`로
돌려준다. 점마다 데이터 단위의 값과 논리 샘플 번호를 함께 기록하므로, 선택 결과를 호스트
데이터로 되돌릴 수 있다.

- 샘플이 4개 이하인 열은 모두 남긴다.
- 그보다 조밀한 열은 첫 샘플, 마지막 샘플, 그리고 그려지는 샘플 가운데 가장 낮은 값(처음
  나온 것)과 가장 높은 값(마지막에 나온 것)을 남긴다. 그려지는 샘플은 같은 열 안에 유한한
  이웃이 있는 유한 샘플, 곧 열 안 선분의 끝점이다. 첫 샘플과 마지막 샘플은 이웃 열과
  이어지므로, 열은 자기 샘플만으로 축약된다.
- 첫 샘플이나 마지막 샘플은 끊김 없이 이어진 구간 안에서만 극값과 잇는다. 두 극값은 항상
  이어서, 그려지는 샘플이 덮는 범위를 열 전체가 덮게 한다. 그래서 한 열 안에서 두 구간을
  가르는, 열보다 좁은 끊김은 이어서 그린다. 화면 해상도로는 구분할 수 없는 끊김이므로 이것이
  맞는 그림이다(관리자 결정, #271). 한 열 이상 넓은 끊김은 선을 끊으며, 끊김이 한 열을 차지할
  때까지 확대하면 보인다.

엔벌로프가 나타내는 꺾은선은 다음과 같다. 셀 안에서는 `links`의 비트 `k`가 켜진 곳에서
점 `k`와 `k + 1`을 잇는다. 한 셀의 마지막 점과 다음 비어 있지 않은 셀의 첫 점은 둘 다
유한하고 샘플 번호가 연속일 때 잇는다(`EnvelopeCell::joins`). 이 꺾은선은 남긴 모든 열에서
전체 선분을 그릴 때와 같은 세로 범위를 덮고, 열마다 들어오고 나가는 샘플을 그대로 지킨다.
단위 테스트가 모든 샘플을 그린 결과와 비교해 확인한다.

누산기는 덧붙이기에 닫혀 있다. 열에 샘플을 여러 번에 나눠 넣어도 한 번에 넣은 결과와 같다.
첫 샘플은 그대로이고, 극값은 갱신되며, 마지막 샘플은 바뀐다. 희소 열은 샘플을 모두 들고
있으므로 마찬가지로 정확히 합쳐진다. 그래서 예전 샘플 없이도 추가 호출을 처리할 수 있다.

엔벌로프는 데이터 단위이므로 y 범위·y 스케일, 색, 선 모양, 제목, 눈금, 범례, 주석이
바뀌어도 플롯 너비만 그대로면 다시 축약하지 않는다.

### 호출

- **전체 호출** `set_monotonic(&source, &mapping)`: 데이터 전체를 `start`, `interval`과 함께
  받아 매번 처음부터 축약한다. 왼쪽 경계 샘플, 열 `0..=width`, 오른쪽 경계 샘플만 읽는다.
  데이터 등록·설정과 매핑 변경은 모두 전체 호출이다.
- **추가** `append_monotonic(&new, MonotonicAppend { sliding: false })`: 새 샘플만 받는다.
  `new`의 샘플 `j`는 논리 샘플 `len + j`이고 `start + (len + j) * interval`에 놓인다.
  `len`은 엔벌로프가 직접 세는 값이다. 아무것도 비교하지 않으며 스케일도 바꾸지 않는다.
  남기는 열에 들어오는 새 샘플은 합치고, 그 뒤 샘플은 세기만 하고 남기지 않는다(오른쪽 경계
  샘플은 예외). 전체 호출 말고는 예전 샘플을 읽지 않으므로 호스트는 오래된 데이터를 버려도
  된다. 채널 수가 다른 소스는 거부한다.
- **밀어내기 추가** `MonotonicAppend { sliding: true }`: 새 샘플을 덧붙이고, x 범위를 정확히
  `new.len() × interval`만큼 옮긴다. 열은 전체 호출이 만든 격자에 고정된 채 다시 나누지
  않는다. 창 왼쪽으로 완전히 벗어난 열은 버리고, 그 열들의 마지막 점이 왼쪽 경계 샘플이
  된다. 새 샘플은 같은 격자의 뒤쪽에 합친다. 범위가 격자보다 앞서 나간 열의 소수 부분은
  `pixel_offset`(`0..1`)으로 알려 주며, 2단계는 그만큼 투영을 옮겨 그린다. 밀어내기는 예전
  샘플을 읽지 않고 전체 축약으로 돌아가지도 않는다. 다만 세기만 하고 남기지 않은 예전
  샘플(남긴 열 너머 두 번째 샘플부터)이 화면에 들어와야 하면 `MonotonicError::SlidePastKept`로
  거부한다.
- **결과**: 모든 호출은 `MonotonicUpdate`를 돌려준다. 논리 길이, `truncated`, 버린 열 수,
  x 축에 반영할 x 범위, 픽셀 오프셋이 담긴다.
- **정렬 확인**: `is_grid_aligned()`는 열이 현재 x 범위에 맞춰져 있는지 알려 준다. 픽셀
  오프셋이 없고, 현재 매핑으로 전체 호출을 해도 모든 샘플이 같은 열에 들어가야 한다. 열
  경계를 계산으로 비교하므로 데이터가 필요 없다. `aligned_mapping()`은 열이 맞춰진 x 범위다.
- **매핑 기록**: 엔벌로프는 그리는 기준이 되는 매핑을 기록하며, 밀어내기는 그 x 범위를 함께
  옮긴다. `check_mapping(current)`는 다른 매핑을 `MonotonicError::SourceRequired { change }`로
  거부한다. 대상은 x 범위, 장치 픽셀 단위 플롯 너비(크기 조정, 플롯 경계를 옮기는 배치 변경,
  DPI 변경, 2배 내보내기), x 축 스케일, 축 끊기, 뒤집기다. 엔벌로프는 데이터를 복사해 두지
  않으므로 스스로 다시 만들 수 없고, 소스를 넘기는 전체 호출만 다시 만들 수 있다.
- **선택**: `column_of_x(x)`는 x가 놓인 열을, `pick(channel, column, y)`는 그 열에 남긴 점
  가운데 `y`에 가장 가까운 점의 논리 샘플 번호와 값을 돌려준다. 둘 다 소스가 필요 없다. 조밀한
  열의 그려진 픽셀은 정확히 이 점들에서 나온다.

축약은 CPU에서 하며, 컴파일러가 벡터화하는 청크 반복문으로 작성했다. SIMD 크레이트는 쓰지
않는다. 연속 구간은 독립 누산기를 둔 블록 단위로 한 번 훑고, 가장 낮은 값(또는 가장 높은
값)이 든 블록만 다시 훑어 위치를 찾는다. 섞인 버퍼 구간은 채널 묶음 단위로 프레임을 차례로
읽으므로, 묶음마다 프레임의 캐시 줄을 한 번만 읽는다. 묶음 크기는 반복 폭일 뿐 제한이
아니다.

### 렌더러에서

렌더러(`figgy-renderer`의 `renderer::monotonic.rs`)는 위 규칙을 모두 지키고 문서, 단조 차트
핸들, GPU 그리기, 상호작용을 더한다.

#### 문서

`DataRenderType::MonotonicLine(MonotonicLineSeries { group, channel, line })`은 묶음의 채널
하나를 선으로 그리는 시리즈다. 채널마다 시리즈가 하나씩이며 각자 id, 라벨, 색, 스타일, 범례
항목을 가진다. 소스는 문서에 들어가지 않는다. 시리즈의 `x_column`과 `y_column`은 렌더러가
그릴 엔벌로프를 담아 두는 풀 컬럼 두 개다. 도우미 `SeriesConfig::monotonic_line(id, group,
channel)`은 이 둘을 `<id>.monotonic.x`, `<id>.monotonic.y`로 짓는다. 두 컬럼은 해당 차트의
그 시리즈 것이다. 호스트는 쓰거나 지울 수 없고(`ReservedColumnId`), 다른 시리즈나 차트가
가진 id 또는 호스트 컬럼과 같은 id를 쓰는 시리즈는 거부한다(`MonotonicColumnTaken`). 선은
직선으로만 잇고 채우지 않는다. x 축은 선형이고 수치형이며 끊김이 없어야 하고 차트는 직교
2D여야 한다. 로그·확률·범주형·끊긴 x 축과 극좌표·삼각·평행좌표·3D 차트는 문서 단계에서
거부한다(`Config::validate_monotonic_series`). 직렬화 형태는
`{"MonotonicLine": {"group": .., "channel": .., "line": {..}}}`라는 새 변형이므로 이전 문서는
그대로 읽고 쓴다.

#### 단조 차트

단조 시리즈가 있는 차트는 따로 등록하는 차트다. `Renderer::register_monotonic_chart(config,
series)`가 `MonotonicChartId`를 돌려주며, 이 핸들로만 차트에 닿는다. 일반 차트 호출
(`set_chart_config`, `set_chart_state`, `set_chart_view_state`, `set_chart_display`,
`hold_chart_display`, 자동 맞춤 계열, 스트리밍 등)은 `ChartId`를 받는데, 단조 차트는
`ChartId`를 내주지 않고 `chart_order`에도 들어가지 않는다. 그래서 어떤 일반 호출도 단조 차트에
닿지 못하고 매핑을 옮길 수도 없다. 거부가 아니라 타입으로 보장한다. 단조 선 시리즈는 단조
차트에만 들어간다. `register_chart`, `set_chart_series`, `set_chart_state`,
`begin_replace_chart_columns`는 등록 단계에서 이를 거부한다(`RuleKind::MonotonicGeneralChart`).
핸들은 내부에서 레지스트리의 차트 id를 감싸므로 단조 차트도 다른 차트와 같은 경로로 그리고
고르고 내보낸다.

`Renderer::monotonic_chart(id)`는 차트 호출 묶음인 `MonotonicChart`를 빌려준다. 매핑을 옮기는
호출은 넷이며 모두 소스를 받는다.

- `set_monotonic(group, &source, range)`: `group`을 가리키는 시리즈를 각자 그려지는 패널의
  매핑으로 처음부터 축약한다. `range`는 `MonotonicRange::X { min, max }`(시리즈가 그려지는 x
  축에 쓰고 축의 나머지는 그대로 둔다) 또는 `MonotonicRange::Autoscale`(아래)이다. x 범위를
  정하는 호출은 이것 하나뿐이다.
- `append_monotonic(group, &new, MonotonicAppend { sliding })`: 새 샘플만 합친다. 밀어내기는
  옮겨진 x 범위를 문서에 쓴다. 부모의 `bottom_x`나 서브플롯 자신의 축(연결된 축이면 연결의 맨
  처음 축)이다.
- `resize(chart_area, plot_area, surface, &[MonotonicGroupRef])`: 크기 조정이다. 차트 영역,
  데이터 영역 크기 조정이 제안한 플롯(데이터 영역, `None`이면 여백 유지), 표시 표면(`None`이면
  표시 없음)을 바꾼다. 시리즈가 다른 장치 픽셀로 옮겨 가는 묶음은 같은 호출 안에서 소스로 다시
  축약한다.
- `fit_layout(&[MonotonicGroupRef])`: 배치를 라벨과 제목에 맞춰(`layout::auto_layout`, 차트
  자신의 글꼴로 잰다) 적용한다. 플롯 경계를 옮길 수 있으므로 시리즈를 옮기는 묶음은 소스로 다시
  축약한다.

나머지 호출은 소스가 필요 없다. `edit_config`(아래), `set_series`(스타일, 채널, 묶음, 시리즈
추가와 삭제), `step_selection`, `set_selection`, `pick_data`, `pick`, `prepare_picking`,
`remove`가 그렇다. 읽기는 `&Renderer`로 한다. `monotonic_config`, `monotonic_series`,
`monotonic_needs_source`(묶음 데이터를 받지 않아 `set_monotonic` 전까지 아무것도 그리지 않는
시리즈, `MappingChange::NotBuilt`), `monotonic_kept_point`(엔벌로프에서 읽은 남긴 점의
`(x, y)`), `monotonic_selection`, `monotonic_display_config`, `monotonic_render_stamp`,
`monotonic_series_draw_info`다. `Renderer::prepare_monotonic`(과
`WindowedRenderer::draw_monotonic`)은 `MonotonicChartDrawItem`을 그리고,
`ExportSource::Monotonic { chart, monotonic: &[MonotonicGroupRef] }`는 단조 차트를 내보낸다.
화면과 같은 매핑이면 `&[]`이다.

**문서 편집기.** `MonotonicChart::edit_config(|editor| ..)`는 문서 사본 위의
`model::monotonic::MonotonicConfig`를 빌려준다. `Config` 전체를 읽을 수 있고(`config()`), 나머지
모든 것을 고칠 수 있다. 범례, 차트 제목, 격자, 주석, 데이터 선택(`picked_data`), 활성 레이어,
배경, 각 축(`MonotonicAxis`: 제목, 눈금 라벨과 형식, 눈금, 간격, 축선, y 축은 범위도),
오버레이 y 축과 범례, 같은 규칙의 서브플롯이 그렇다. x 범위, x 스케일, 끊기, 방향, 차트·플롯
영역을 쓰는 메서드는 없고 `&mut Config`도 내주지 않는다. 꾸밈 요소 끌기는 `nudge`와 `drag`로
하며, 매핑을 바꿀 움직임(플롯 너비를 바꾸는 축이나 플롯 가장자리, 서브플롯 틀)은 거절한다.
`assign(config)`는 문서 전체를 바꾸며(데이터로 편집한 문서) 매핑을 옮기는 문서는 거부한다.
렌더러는 고친 사본을 `same_mapping`(문서 크기, 패널마다 플롯 너비와 x 범위·스케일·끊기·방향·
범주)으로 확인한 뒤에만 받아들이고, 어긋나면 `RuleKind::MonotonicMappingEdit`로 거부하며
아무것도 바꾸지 않는다.

**차트를 바꾸는 호출은 모두 공통 단계 하나를 거친다.** 이 단계는 호출 뒤의 문서를 차트 표시가
보여 줄 배치로 놓고 단조 시리즈마다 계획을 세운다. 매핑이 그대로인 시리즈는 묶음의 엔벌로프를
그대로 쓰고, 호출이 소스를 빌려준 묶음은 시리즈가 그려지는 모든 매핑에서 다시 축약한다. 소스
없이 묶음을 옮기는 크기 조정이나 배치 맞춤은 아무것도 쓰기 전에
`FiggyError::MonotonicSourceRequired { series_id, change }`로 거부되며, 문서, 표시, 엔벌로프,
그림은 모두 그대로다. 크기 조정은 표시 표면과 문서를 함께 계획하므로 원자적이다. 거부되면
아무것도 바뀌지 않고, 그 뒤에 실패하면 표시를 되돌린다. 여백으로 놓을 수 없는 플롯 사각형은
`RuleKind::NoFeasibleLayout`으로 거부한다. 플롯 가장자리 바깥 여백은 엔벌로프를 그리는 배율에서
64픽셀 열로 고정이므로(`MARGIN_PX`) 선 굵기는 일반 스타일 수정이다. 프레임과 선택은 소스 때문에
실패하지 않는다. 엔벌로프는 언제나 그려지는 매핑과 맞는다. 다른 배율로 내보낼 때는 소스를 넘기며,
내보내기 매핑으로 축약한 임시 컬럼은 그 내보내기에서만 쓴다.

**자동 배율은 전체 호출의 범위 가운데 하나다.** `set_monotonic(group, &source,
MonotonicRange::Autoscale)`은 묶음의 시리즈가 그려지는 축의 x 범위를 소스 격자 계산으로
`start ..= start + (len - 1) × interval`에 맞추고(자동 맞춤처럼 눈금 계획도 함께 맞춘다), y
범위를 각 시리즈의 엔벌로프가 플롯 안에 남긴 가장 낮은 값과 가장 높은 값에 맞춘 뒤 양쪽에 폭의
5 %씩 여유를 둔다. 이 값은 같은 호출의 축약에서 나온다. 축약은 열마다 극값을 남기므로 데이터를
다시 읽지 않는다(표시 값, 규칙 1). 기본 레이어 시리즈는 패널의 `left_y`(짝 축 포함)를, 오버레이
시리즈는 오버레이 자신의 축을, 서브플롯 시리즈는 서브플롯의 축(x는 연결의 맨 처음 축)을 맞춘다.
폭이 없는 범위(샘플 하나, 일정한 채널)는 축을 그대로 둔다. 일반 자동 맞춤은 `ChartId`를 받으므로
단조 차트에 닿지 못한다.

#### 그리기

시리즈의 엔벌로프는 풀 컬럼 두 개이며, 렌더러의 컬럼 upsert로 올린다(`ColumnSource`가 매핑된
스테이징에 바로 쓴다). 그리기는 기존 선 파이프라인(`line_columnar.wgsl`과 `common/`의 선
조각)이 맡는다. 새 셰이더, 진입점, 파이프라인은 없다. 슬롯마다 8행을 쓴다. 점 4개 각각 뒤에
이음 표시가 붙는데, 선이 이어지면 점을 한 번 더 쓰고 끊기면 NaN을 쓴다. 같은 점이 두 번이면
길이가 0인 선분이 되어 아무것도 그리지 않고, NaN 행은 다른 선과 마찬가지로 닿는 선분을
가린다. 그래서 행 번호에서 슬롯과 점을 계산으로 찾는다(`row / 8`, `(row % 8) / 2`). 선택된
행을 논리 샘플 번호로, 선택한 번호를 행으로 바꾸는 방법이 이것이다. x는 데이터 단위의
`start + index × interval`이므로 밀어낸 오프셋과 상관없이 남긴 점은 전체를 그릴 때와 정확히
같은 자리에 놓인다. 채널마다 프레임당 `8 × (w + 3)`행을 그리며 샘플 수와 무관하다. 그중 절반
정도는 길이가 0이거나 가려진 행이다. 엔벌로프는 그리는 배율에서 양쪽 바깥으로 64픽셀 열을
고정 여백으로 더 둔다(`MARGIN_PX`). 선이 플롯 안으로 들어오는 바깥 열이다. **한계:** 장치
픽셀로 약 120픽셀(선 굵기에 표시 배율이나 내보내기 배율을 곱한 값)까지의 선은 가장자리를 전체
그리기와 똑같이 그린다. 더 굵은 선은 플롯 가장자리의 잉크를 놓칠 수 있다. 이 여백은 데이터 개수가
아닌 표시 값(규칙 1)이고 어떤 스타일 수정도 바꾸지 않으므로, 선 굵기를 바꿔도 소스가 필요 없다.
단조 선에는 마커가 없다(`strokes_up_to_the_margin_limit_draw_their_edges_exactly`: 40, 60픽셀 선을
1배와 2배로, 즉 장치 픽셀 120까지 확대한 뒤 전체 그리기와 바이트까지 같다).

#### 픽셀(화면 해상도의 M2)

`tests/monotonic_render.rs`는 단조 시리즈를 그린 차트와 같은 샘플을 보통 선으로 그린 차트를
비교한다.

- **희소한 보기**(픽셀 열마다 보이는 샘플 4개 이하)는 샘플을 모두 남기므로 1배와 2배에서
  바이트까지 같다.
- **조밀하거나 섞인 보기**는 열의 첫 샘플, 가장 낮은 샘플, 가장 높은 샘플, 마지막 샘플로 열의
  범위를 그린다. 바이트까지 같지는 않다. 전체 그리기는 샘플마다 안티에일리어싱된 선을 하나씩
  쌓고, 선 가장자리가 샘플이 열 안 어디에 있든 이웃 열까지 번지기 때문이다. 엔벌로프는 선을
  네 개만 그린다. 두 그림 가운데 한쪽이 칠한 픽셀은 모두 다른 쪽이 칠한 픽셀에서 1픽셀 안에
  있다. 1배와 2배, 가는 선과 굵은 선에서 모두 확인한다.
- **추가와 열 단위 밀어내기**는 같은 창을 전체 호출한 결과와 바이트까지 같고, **열보다 작은
  밀어내기**는 전체 그리기와 1픽셀 안에서 같다.
- **열보다 좁은 NaN 끊김**은 이어서 그리고, 더 넓은 끊김은 두 그림 모두 선을 끊는다.
- **굵은 선**도 한계 안에서는 가장자리가 같다. 40, 60픽셀 선을 희소한 창으로 확대하면 1배와
  2배에서(장치 픽셀 120까지) 전체 그리기와 바이트까지 같다. 더 굵은 선은 가장자리 잉크를 놓칠 수
  있다.

#### 상호작용(M6)

- **선택**은 다른 선처럼 그려진 엔벌로프 위에서 GPU로 하며, 남긴 점의 논리 샘플 번호를
  알려 준다. 티켓은 제출할 때의 엔벌로프로 행을 번호로 되돌린다. 조밀한 열의 그려진 픽셀은
  정확히 이 점들에서 나온다.
- **선택 강조**는 선택한 논리 번호의 남긴 점에 고리를 그린다. 엔벌로프가 남기지 않은 샘플은
  고리가 없다.
- **`step_selection`**은 남긴 점을 따라 옮긴다(`MonotonicChart::step_selection`).
- **범례**: 시리즈 색의 선 기호다. 테마 색 순환이 색을 바꾼다.
- **레이어**: 오버레이(부모의 x 축, 오버레이의 y 축)와 서브플롯(자기 x 범위와 너비, 묶음이
  그려지는 매핑마다 엔벌로프 하나)을 지원한다.
- **꾸밈 요소와 플롯**: 제목, 범례, 축 제목, 눈금 라벨 끌기와 데이터 영역 이동은 편집기의
  `nudge`와 `drag`로 한다. 데이터 영역 크기 조정은 크기 조정(`MonotonicChart::resize`에 플롯
  사각형을 넘김)이므로 소스를 받는다.
- **내보내기**: PNG, SVG, PDF를 어떤 배율로도 내보내며, 매핑이 다르면 소스를 함께 넘긴다.
- **장면 목록**: `tests/support/playground_scenes.rs`의 `SceneKind::Monotonic`(채널 넷, 각
  200 000개 샘플)은 데코레이션 오라클, 벡터 내보내기 점검, `auto_layout` 점검에서 모두 돈다.
  장면은 샘플을 직접 들고 있고(`CartesianScene::monotonic`), 점검은 `export_scene`으로
  내보낸다. 이 함수는 그 호출 동안 문서를 단조 차트로 등록하고 샘플을
  `ExportSource::Monotonic`에 넘긴다. 플레이그라운드는 단조 장면을 단조 차트로 등록하고, 장면의
  샘플과 함께 `MonotonicChart::resize`로 화면 크기를 따라가며, 데이터를 한 번도 받지 않은 묶음에
  처음 전체 호출을 하고, 문서는 편집기로 고치며, 이동·확대·고정하지 않는다.
- **스트리밍과 무관**(M5): 스트리밍 표시에 든 단조 시리즈는 거부한다(`StreamedMonotonic`).

브라우저 제품도 핸들을 따른다. `FiggyChart.set_monotonic_document`는 단조 차트를 보여 주며,
그 설정 호출은 편집기로 문서를 고친다. 단조 호출은 원소 형식 여섯 가지의 타입 배열을 채널마다
하나씩, 또는 섞인 배열 하나와 간격(stride)으로 받는다(`set_monotonic`, `append_monotonic`,
`resize_monotonic`, `fit_layout`, `needs_source`, `monotonic_groups`,
`export_png_monotonic`과 SVG·PDF 형태). 휠과 끌기 확대는 쓰지 않고, 데이터 영역 크기 조정
몸짓은 호스트가 적용할 사각형을 제안한다. `WASM.md`를 참고한다.

### 이 소스의 규칙(M1–M7)

단조 선 시리즈에만 적용하는 별도 규칙이다. 저장소 `CLAUDE.md`의 규칙 1, 2, 5, 8은 다른
시리즈에 그대로 적용된다.

- **M1. 입력.** 데이터는 값과 `start`, `interval`을 함께 담은 `MonotonicSource`로만
  들어오며, 데이터를 읽는 호출마다 넘긴다. `start`나 `interval`을 따로 받는 호출은 없다.
  다른 컨테이너나 `ColumnSource` 업로드를 쓰지 않는다.
- **M2. 축약은 외형이다.** 렌더러는 픽셀 열마다 M4로 줄여도 된다. M2는 화면 해상도 기준이다.
  전체 호출 뒤나 열 단위 변경 뒤에는 열마다 모든 선분을 그릴 때와 같은 범위를 덮는다(희소한
  보기는 샘플을 모두 남겨 같은 그림을 그린다). 열보다 작게 밀어내는 동안에는 고정 격자와
  오프셋 때문에 픽셀 한 열 미만만큼 다를 수 있으며, 다음 전체 호출이 정확히 다시 맞춘다. 열보다
  좁은 끊김은 구분하지 않는다.
- **M3. 캐시 한도.** CPU 캐시에는 열 누산기(열마다 샘플 4개, 크기는 플롯 너비와 채널 수로
  정해지며 데이터 길이와 무관)와 만들 때의 스칼라 값만 둔다. 데이터 복사본은 두지 않는다.
- **M4. 재계산.** 전체 호출은 처음부터 축약한다. 새 샘플을 합치는 것은 `append`뿐이고, 앞쪽
  열을 버리는 것은 밀어내기 `append`뿐이다. 추가하는 동안 스케일은 바뀌지 않는다. 매핑은 단조
  차트 핸들에서 소스를 받는 호출 넷(`set_monotonic`, `append_monotonic`, `resize`,
  `fit_layout`)만 옮긴다. 일반 차트 호출은 단조 차트에 닿지 못하고, 문서 편집기는 매핑을 옮길
  수정을 거부한다.
- **M5. 스트리밍과 무관.** 이 소스는 기존 스트리밍 런타임을 쓰지도, 늘리지도, 의존하지도
  않는다.
- **M6. 상호작용.** 규칙 6을 모두 지킨다. 마우스 올림, 클릭 선택, `step_selection`, 선택
  강조는 남긴 점의 논리 샘플 번호를 소스 없이 알려 준다. 선택은 다른 선과 마찬가지로 포인터
  아래 그려진 선분에서 가까운 쪽 끝의 남긴 점이다.
- **M7. 개수 제한 없음.** 규칙 10을 지킨다. 샘플, 채널, 열, 간격 수에 상한을 두지 않는다.
