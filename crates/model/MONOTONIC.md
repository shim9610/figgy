# Monotonic-x line core / 단조 x 선 코어

The CPU core of the monotonic-x line chart (#271), in
`model::monotonic`. It serves fast, large, evenly spaced data:
`x = start + i * interval`, strictly increasing. Per pixel column it reduces
the samples to **M4**: the first and last samples, and the lowest and
highest. A renderer then draws about `4 × plot width` vertices per channel
however many samples there are. It draws lines only: scatter is out of
scope, because keeping the extremes per column drops the points between
them.

This phase is the pure-CPU core. The renderer, the GPU draw, the
interactions and the WASM wrappers come in phase 2.

## Source

The host lends its samples through the `MonotonicSource` trait, for one call
at a time; the envelope keeps no reference to it. A source has `len()`
samples per channel and `channel_count()` channels, and lends a requested
range as contiguous runs (`MonotonicRun`), in order, through a callback,
without allocating. A run is either one slice per channel or frames of an
interleaved buffer, both read in place, so the reduction gets contiguous
memory to vectorize. A ring buffer lends a wrapped range as two runs.

| Built-in impl | Channel `n`, sample `i` |
|---|---|
| `MonotonicSlices::new(&[&[T]])` | `slices[n][i]` |
| `MonotonicInterleaved::new(values, stride, columns)` | `values[i * stride + columns[n]]` |

In an interleaved buffer a column no channel names (a timestamp, say) is
never read, and a trailing partial frame is not counted until it is
complete. When per-channel slices differ in length, the source lends their
common extent and reports `truncated`.

The element type is one of `f64`, `f32`, `i32`, `i16`, `u16` and `u8`
(`MonotonicElement`, sealed), and every type goes through the same generic
path. The reduction compares in the element type and converts only the kept
values to `f64`; nothing is converted or copied in bulk. A non-finite float
(NaN or ±infinity) is a gap in the line. Integers have no gaps.

A `MonotonicGroup` is the group of a full call: a source and its x grid,
`start` and `interval > 0`.

## Columns

The mapping (`MonotonicMapping`: x range, plot width in device pixels, x
scale, x breaks, x inversion) places sample `i` at
`t = (start + i * interval - x_min) / x_per_pixel`, with
`x_per_pixel = (x_max - x_min) / width`. Column `c` holds the samples with
`c <= t < c + 1`. Every boundary is plain arithmetic on these scalars,
corrected by a step or two where rounding needs it, never a search over the
data (`MonotonicGrid`). A logarithmic, probability or broken x axis is not
meaningful for this source and is refused with
`MonotonicError::UnsupportedXMapping`.

Per channel the envelope keeps `width + 3` slots:

- the lead edge sample, the last sample left of column 0;
- columns `0..width`;
- column `width`, just right of the plot, kept in full so that a slide by
  part of a column can show it;
- the trail edge sample, the first sample right of column `width`.

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
  runs inside that column is therefore drawn closed. Whether that may stay
  is open with the maintainer.

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
| `set_monotonic(&group, &mapping)` | the whole data, `start`, `interval` | the lead edge, columns `0..=width` and the trail edge, nothing else |
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

## Rules for this source (M1–M7)

A separate rule set for monotonic line series only. Rules 1, 2, 5 and 8 of
the repository's `CLAUDE.md` keep governing every other series.

- **M1. Input.** Data enters only through `MonotonicSource`, passed to every
  call, with `start` and `interval` on the full call. No other container,
  and no `ColumnSource` upload.
- **M2. Reduction is appearance.** The renderer may reduce to M4 per pixel
  column. After a full call or a whole-column change, the drawn image equals
  drawing every segment. While sliding by fractions of a column, it differs
  by less than one pixel column, through the anchored grid and its offset;
  the next full call realigns it exactly.
- **M3. Cache bound.** The CPU cache holds only the column accumulators
  (4 kept samples per column, sized by plot width and channel count, never
  by data length) and the scalars they were built with. Never a copy of the
  data.
- **M4. Recompute.** The full call reduces from scratch. Only `append`
  merges new samples, and only a sliding `append` drops columns from the
  front. The scale never changes inside an append.
- **M5. Not streaming.** This source does not use, extend or depend on the
  streaming runtime, which stays as it is.
- **M6. Interactions.** Rule 6 applies in full. Hover, click selection,
  `step_selection` and the picked highlight report the logical sample index
  of the nearest kept point in the column under the pointer, without the
  source.
- **M7. No count limits.** Rule 10 applies: no cap on samples, channels,
  columns or stride.

## 한국어

`model::monotonic`은 단조 x 선 차트(#271)의 CPU 코어다. 간격이 일정하고
(`x = start + i * interval`) 빠르게 늘어나는 대용량 데이터를 다룬다. 픽셀 열마다 샘플을
**M4**로 줄인다. 첫 샘플과 마지막 샘플, 가장 낮은 값과 가장 높은 값만 남긴다. 그래서
샘플이 몇 개든 렌더러는 채널마다 정점을 `4 × 플롯 너비` 개 정도만 그린다. 선만 지원한다.
산점도는 열마다 극값만 남기면 그 사이 점이 사라지므로 대상이 아니다.

이번 단계는 CPU 코어만 구현한다. 렌더러, GPU 그리기, 상호작용, WASM 래퍼는 2단계에서
다룬다.

### 데이터 소스

호스트는 `MonotonicSource` 트레이트로 샘플을 호출마다 빌려준다. 엔벌로프는 호출이 끝나면
소스 참조를 남기지 않는다. 소스는 채널마다 `len()`개 샘플과 `channel_count()`개 채널을
가지며, 요청받은 범위를 연속 구간(`MonotonicRun`)으로 나눠 순서대로 콜백에 넘긴다.
메모리를 따로 할당하지 않는다. 구간은 채널별 슬라이스이거나 섞인 버퍼의 프레임이며, 둘 다
제자리에서 읽으므로 축약이 연속 메모리를 벡터화할 수 있다. 링 버퍼는 한 바퀴 돈 범위를 두
구간으로 넘긴다.

- 채널별 슬라이스: `MonotonicSlices::new(&[&[T]])`. 채널 `n`의 샘플 `i`는 `slices[n][i]`다.
- 샘플 순서로 섞인 버퍼: `MonotonicInterleaved::new(values, stride, columns)`. 채널 `n`의
  샘플 `i`는 `values[i * stride + columns[n]]`이다.

섞인 버퍼에서 어느 채널도 지정하지 않은 열(타임스탬프 등)은 읽지 않으며, 끝에 덜 채워진
프레임은 다 찰 때까지 세지 않는다. 채널별 슬라이스의 길이가 다르면 공통 길이만 빌려주고
`truncated`를 알린다.

원소 형식은 `f64`, `f32`, `i32`, `i16`, `u16`, `u8` 가운데 하나이며(`MonotonicElement`),
모든 형식이 같은 제네릭 코드를 지난다. 비교는 원래 형식으로 하고 남기는 값만 `f64`로
바꾼다. 데이터를 통째로 변환하거나 복사하지 않는다. 유한하지 않은 부동소수(NaN, ±무한대)는
선의 끊김이며, 정수에는 끊김이 없다.

`MonotonicGroup`은 전체 호출에 넘기는 묶음으로, 소스와 그 x 격자(`start`, `interval > 0`)를
담는다.

### 열

매핑(`MonotonicMapping`: x 범위, 장치 픽셀 단위 플롯 너비, x 축 스케일, 축 끊기, 뒤집기)은
샘플 `i`를 `t = (start + i * interval - x_min) / x_per_pixel` 위치에 놓는다. 여기서
`x_per_pixel = (x_max - x_min) / width`다. `c <= t < c + 1`인 샘플이 열 `c`에 속한다. 열
경계는 모두 이 값들로 계산하고 반올림 오차는 한두 단계 보정으로 맞추며, 데이터를 탐색하지
않는다(`MonotonicGrid`). 로그·확률·끊긴 x 축은 이 소스에 의미가 없으므로
`MonotonicError::UnsupportedXMapping`으로 거부한다.

엔벌로프는 채널마다 슬롯 `width + 3`개를 둔다.

- 왼쪽 경계 샘플: 열 0 왼쪽의 마지막 샘플
- 열 `0..width`
- 열 `width`: 플롯 바로 오른쪽 열. 열보다 작은 밀어내기에서 보일 수 있으므로 통째로 둔다.
- 오른쪽 경계 샘플: 열 `width` 오른쪽의 첫 샘플

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
  가르는, 열보다 좁은 끊김은 막혀 보인다. 이대로 둘지는 관리자와 정할 문제로 남아 있다.

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

- **전체 호출** `set_monotonic(&group, &mapping)`: 데이터 전체와 `start`, `interval`을
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

### 이 소스의 규칙(M1–M7)

단조 선 시리즈에만 적용하는 별도 규칙이다. 저장소 `CLAUDE.md`의 규칙 1, 2, 5, 8은 다른
시리즈에 그대로 적용된다.

- **M1. 입력.** 데이터는 `MonotonicSource`로만 들어오며 호출마다 넘긴다. `start`와
  `interval`은 전체 호출에 넘긴다. 다른 컨테이너나 `ColumnSource` 업로드를 쓰지 않는다.
- **M2. 축약은 외형이다.** 렌더러는 픽셀 열마다 M4로 줄여도 된다. 전체 호출 뒤나 열 단위
  변경 뒤에는 모든 선분을 그린 결과와 같아야 한다. 열보다 작게 밀어내는 동안에는 고정 격자와
  오프셋 때문에 픽셀 한 열 미만만큼 다를 수 있으며, 다음 전체 호출이 정확히 다시 맞춘다.
- **M3. 캐시 한도.** CPU 캐시에는 열 누산기(열마다 샘플 4개, 크기는 플롯 너비와 채널 수로
  정해지며 데이터 길이와 무관)와 만들 때의 스칼라 값만 둔다. 데이터 복사본은 두지 않는다.
- **M4. 재계산.** 전체 호출은 처음부터 축약한다. 새 샘플을 합치는 것은 `append`뿐이고, 앞쪽
  열을 버리는 것은 밀어내기 `append`뿐이다. 추가하는 동안 스케일은 바뀌지 않는다.
- **M5. 스트리밍과 무관.** 이 소스는 기존 스트리밍 런타임을 쓰지도, 늘리지도, 의존하지도
  않는다.
- **M6. 상호작용.** 규칙 6을 모두 지킨다. 마우스 올림, 클릭 선택, `step_selection`, 선택
  강조는 포인터 아래 열에 남긴 점 가운데 가장 가까운 점의 논리 샘플 번호를 소스 없이
  알려 준다.
- **M7. 개수 제한 없음.** 규칙 10을 지킨다. 샘플, 채널, 열, 간격 수에 상한을 두지 않는다.
