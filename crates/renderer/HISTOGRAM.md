# Histogram orientation / 히스토그램 방향

`DataRenderType::Histogram` already supports both directions through
`DataBarStyleConfig::orientation`. This is the existing column-based renderer;
it does not use the bounded categorical chart contract. The host supplies bin
edges and values. The renderer does not bin raw observations.

| Orientation | `x_column` | `y_column` | X axis | Y axis |
|---|---|---|---|---|
| `Vertical` | edges | counts | Bin edges | Count |
| `Horizontal` | counts | edges | Count | Bin edges |

For N bins, provide N+1 edges and N counts. Orientation determines the roles;
column length is never used to guess them. Counts may also represent weighted
values; label the value axis accordingly. Each bin's style override and selection
remain keyed by its bin index in either direction.

## Existing chart: vertical → horizontal

The following assumes a vertical histogram registered as `chart_id`, with edges
bound to X and counts bound to Y. Change the bindings, direction and axis options
together. No data copy or re-upload is needed.

```rust,ignore
use renderer::data_config::{BarOrientation, DataRenderType};

let mut config = renderer.chart_config(chart_id)?.clone();
let mut series = renderer.chart_series(chart_id)?.to_vec();
for s in &mut series {
    let DataRenderType::Histogram { bar } = &mut s.render_type else {
        return Err("this example expects a histogram-only chart".into());
    };
    if bar.orientation == BarOrientation::Vertical {
        std::mem::swap(&mut s.x_column, &mut s.y_column);
        bar.orientation = BarOrientation::Horizontal;
    }
}
// Set the count-axis range/scale/title on bottom_x, and the bin-axis
// range/scale/title on left_y. Here the existing linear example uses:
config.bottom_x.min = 0.0;
config.bottom_x.max = 120.0;
config.left_y.min = -4.2;
config.left_y.max = 4.2;
config.bottom_x.title_option.text.segments = renderer::text::rich_segments_from_text("Count");
config.left_y.title_option.text.segments = renderer::text::rich_segments_from_text("Measured value");
renderer.set_chart_state(chart_id, config, series)?;
// Refresh the ChartView raster and prepare_registered for the next paint,
// as after any other Config edit. Orientation does not request auto-fit.
```

Axis scale, inversion, tick format/spacing and grid direction remain explicit
Config choices. Adapt these as well when exchanging axis roles; do not exchange
the entire axis structs because their placement/margins belong to screen sides.
For a horizontal logarithmic count axis, set `bottom_x.scale` to `Logarithmic`
and use a positive range. The usual zero baseline is clipped to the visible
axis minimum. Nonpositive values have no logarithmic position.

For native PNGs in both directions, run:

```bash
cargo run --locked -p figgy-renderer --example readme_gallery -- target/histogram-preview 2
```

Inspect `gallery-histogram.png` and `gallery-histogram-horizontal.png`.
The example uploads the data once and reuses those columns. It also writes
the other gallery PNGs and two frames of each animation.

## 한국어 사용법

가로 히스토그램은 `bar.orientation = BarOrientation::Horizontal`로 설정한다.
이때 X컬럼은 빈도, Y컬럼은 구간 경계다. 세로 히스토그램과 컬럼 연결이 반대이므로
기존 차트에서 방향만 바꾸면 안 된다. 위 예제처럼 컬럼 연결과 방향, 축 범위·제목을
함께 바꾸고 `set_chart_state`로 반영한다. 데이터는 다시 올릴 필요가 없다.

구간이 N개면 경계는 N+1개, 빈도는 N개를 전달한다. 집계는 호스트가 담당하며,
일반 컬럼 데이터 소스를 그대로 사용한다. 소규모 범주형 막대의 별도 입력 계약과는 다르다.
방향 변경이 자동 맞춤을 요청하지는 않는다. 축 눈금·로그 설정·뒤집기·그리드도
용도에 맞게 호스트에서 지정한다. 보조 축을 표시한다면 해당 축도 함께 설정한다.

외곽선은 `border_width = 0`이면 끄고, 양수이면 해당 두께로 그린다.
`width_ratio`와 `gap_px`는 가로 방향에서 각 구간의 세로 두께와 간격에 적용된다.
`bar_style_overrides`와 선택 상태의 `bin_index`는 방향을 바꿔도 같은 구간을 가리킨다.

## Verification

`histogram_render` tests the horizontal endpoints, logarithmic count axis at
1×/1.5×/2×, signed values, unequal bins, zero values, per-bin width/color and
selection outline, alongside the existing vertical and subpixel-envelope cases.
The renderer SSOT test changes vertical → horizontal → vertical after painting,
compares each first updated frame with a fresh renderer, checks that the column
pool is unchanged, and checks style reuse for an identical repeated update.

```bash
cargo test --locked -p figgy-renderer --test histogram_render
cargo test --locked -p figgy-renderer --lib histogram_orientation_round_trip
```
