# Categorical bars / 범주형 막대 차트

A categorical bar chart is a Cartesian chart. Put one
`DataRenderType::CategoryBars` series per bar series on a `Config` with a
categorical axis (`AxisOptions::categories`), and the Cartesian `Renderer`
draws them. Positions and values are read from columns uploaded through
`ColumnSource`; streamed sources are rejected. `CategoricalChart` is the
document format of the releases up to 0.12, and `Config::from_categorical`
reads such a document as a categorical bar chart
([Opening as a Cartesian chart](#opening-as-a-cartesian-chart)). The rules
below apply to this document. `CategoricalChart` is kept only as a deprecated
document reader in this release and is removed in the next milestone.

```rust,no_run
use renderer::{CategoricalChart, Category, BarSeries, Color, CategoryBarMode};
let chart = CategoricalChart {
    categories: vec![Category::new("a", "A"), Category::new("b", "B")],
    series: vec![
        BarSeries::new("before", "Before", vec![Some(12.0), Some(8.0)], Color::from_rgb8(70,130,205)),
        BarSeries::new("after", "After", vec![Some(18.0), Some(15.0)], Color::from_rgb8(255,166,105)),
    ],
    mode: CategoryBarMode::Grouped,
    ..Default::default()
};
chart.validate()?;
# Ok::<(), &'static str>(())
```

## Input and stacking rules

- There is at least one category and at least one series. The number of
  categories, the number of series and `categories × series` have no upper
  limit.
- Category and series IDs must be unique within their lists. Names may
  repeat, and renaming does not change the ID.
- Each series' `values` has as many entries as there are categories. `None`
  is missing and `Some(0.0)` is a real 0. Neither has bar area, but 0 stays
  in the value, the label and the selection state. The pointer selects only
  bars that have area.
- `Grouped` draws the bars side by side. With one series it is a single bar.
  A missing value does not move the other bars.
- `Stacked` stacks positive values up/right and negative values down/left,
  each from 0.
- `PercentStacked` scales the positive total of each category to 100%.
  Negative values are rejected. A total of 0 is an empty bar, and its percent
  label is `—`. The original values are not changed. Very large values do
  not overflow the total.
- In plain stacking, a total outside the finite number range is rejected.
  NaN and infinity are rejected in every mode.
- The `Percent` label is relative to the total of the series in the same
  category. A category with negative values shows `—`.
- The value axis is linear. `Chart::auto_fit_layer_union` fits it to 0 and
  the range the bars draw. Stacked bars fit to
  `[Σ min(0, minimum), Σ max(0, maximum)]` from the minimum and maximum
  stored for each value column. When a series' extremes fall in different
  categories, the upper bound is wider than the actual stack. Uploading the
  columns at once or separately gives the same result. 100% stacking fits to
  `0..100`.
- Columns the host uploads directly follow the same rules. If a value column
  of a `PercentStacked` group has a negative value (the column minimum is
  below 0), drawing and picking reject it with an `InvalidSeriesConfig` that
  names the series and the mode. `Stacked` and `Grouped` draw negative
  values.

Selection and per-bar styles refer to IDs, so they still point at the same
item after a rename. When an item is deleted, delete its selection and
per-bar settings too.

## Direction, style and per-bar editing

`direction` is `Vertical` / `Horizontal`, and changing it needs no change to
the input values or the category array. `value_title` and `category_title`
also move with each axis' role. On a polar chart (`Config::polar`, with a
categorical angle axis) the same series becomes a rose: it draws wedges
radially within each category's angular cell, and only `Vertical` is allowed
([POLAR.md](https://github.com/shim9610/figgy/blob/master/crates/renderer/POLAR.md#roses-polar-bars)).

Styles apply in the order **chart → series → single bar**. `style: Some(...)`
replaces the whole style and no longer inherits later changes to the parent
style. Setting it back to `None` inherits again. The color comes from the
series' `color`, and a single bar's color from `CategoryBarOverride.color`.

```rust,no_run
use renderer::{CategoricalChart, CategoryBarTarget, CategoryBarOverride,
    CategoryBarStyle, CategoryBarLabels, CategoryBarLabelFormat, Color, Material};
# fn edit(chart: &mut CategoricalChart) {
let target = CategoryBarTarget::new("a", "before");
let mut edit = CategoryBarOverride::new(target.clone());
edit.color = Some(Color::from_rgb8(30, 175, 165));
edit.style = Some(CategoryBarStyle {
    material: Material::SatinMetal,
    outline: true,
    ..chart.style.clone()
});
edit.labels = Some(CategoryBarLabels::Inside);
edit.label_format = Some(CategoryBarLabelFormat::ValuePercent);
// A target can have only one override, so remove the existing one and add the new one.
chart.overrides.retain(|o| o.target != target);
chart.overrides.push(edit);
chart.selected = Some(target);
# }
```

| Option | Default | Range and behavior |
|---|---|---|
| `material` | `Flat` | Any [`Material`](RADIAL.md#materials) except `Toon` and `Pearl`. A bar is shaded like a flat pie top |
| `corner_radius` | 5 | 0 to 32 logical pixels. Limited to half the bar's thickness and length |
| `outline` | false | Turns on the outline |
| `outline_width` | 1 | 0 to 8 logical pixels. Drawn inside the shape |
| `outline_color` | RGB(40,55,75) | Each RGBA component 0 to 1, alpha allowed |
| `texture_strength` | 0.2 | 0 to 1; 0 turns the pattern off |
| `texture_scale` | 1 | 0.1 to 8; higher values make the pattern finer |
| `gloss` | 0.45 | 0 to 1. The sheen of `SatinMetal` and `BrushedMetal`; it has no visible effect on `Enamel` bars (see [Materials](RADIAL.md#materials)) |
| `emphasis_brightness` | 0.08 | 0 to 0.5, brightness increase on hover and selection |
| `group_width` | 0.7 | The fraction of a category cell the bars take, 0.1 to 0.95 |
| `bar_gap` | 3 | Logical pixels between bars in a group, 0 to 24 |
| `grid`, `legend` | true | Show the grid and the series legend |

Grouped bars round only the end away from 0. Stacked bars round only the end
of the last nonzero segment of each positive and negative stack, and keep the
inner boundaries straight. The original values and the bar end positions do
not change with material, outline or hover. Textures need no external
texture files. This is a 2D look for comparing category colors; the radial
chart's 3D camera and extrusion are not applied.

## Labels and fonts

- `labels`: `None` / `Inside` / `Outside` / `Auto` (default). A per-bar
  `labels: None` inherits, and `Some(CategoryBarLabels::None)` hides the
  label.
- `Auto`: grouped bars show labels outside, stacked bars inside.
- `label_format`: `Value` (default) / `Percent` / `ValuePercent` /
  `CategoryValue`. 100% stacking does not switch value labels to percentages
  by itself; state the format you want.
- `label_decimals`: 0 to 6 (default 1); trailing zeros are removed.
- `value_suffix`: one line of up to 24 characters. Include the space, as in
  `" kg"`.
- `font_size`: 8 to 32 logical pixels (default 14); `font_family`: default
  `sans-serif`.
- Category and series names and IDs are up to 80 characters without control
  characters; titles and axis titles are up to 160 characters.

The reader moves these settings into the series' `labels`
(`CategoryBarSeriesLabels`) and `label_format`, and `format_labels` writes
the string to show for each bar into `labels.text`. The renderer does not
format values; it draws these strings at the bar ends. Outside labels are
placed 6px past the bar end and inside labels in the middle of the bar.
`Auto` draws grouped bars' labels outside, and on stacked bars draws inside
only the segments the label fits in. Text inside a dark bar is white;
otherwise the font, size and color of the value axis' tick labels are used.
Empty strings are not drawn, and labels do not push each other apart. When
an outside label crosses the edge of the data area, it is drawn inside the
bar end. `pick_chart_data` picks labels as `PickedData::ValueLabel`, and
dragging one records the offset in the series' `labels.offsets`. `DataLabel`
offsets of the original document are also moved into `labels.offsets` (see
Value labels in the [interaction guide](INTERACTION.md)).

## Opening as a Cartesian chart

```rust,no_run
use renderer::{CategoricalChart, Chart, ColumnSource, LayerRef, Renderer};
# fn open(r: &mut Renderer, json: &str) -> Result<(), Box<dyn std::error::Error>> {
let document: CategoricalChart = serde_json::from_str(json)?;
document.validate()?;
let (mut config, series, uploads) = renderer::config::Config::from_categorical(&document);
config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
    x: 0, y: 0, width: 900, height: 620,
});
// Upload the position column first and the value columns at once. The uploads borrow the document's values.
let (positions, values) = uploads.split_first().expect("positions");
r.add_column(positions.column.clone(), &positions.values)?;
let columns: Vec<(&str, &dyn ColumnSource)> = values
    .iter()
    .map(|u| (u.column.as_str(), &u.values as &dyn ColumnSource))
    .collect();
r.add_columns(&columns)?;
let mut chart = Chart::new(config);
chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
# Ok(())
# }
```

`Config::from_categorical` moves the categories to the categorical axis that
fits the direction (`bottom_x` for vertical bars, `left_y` for horizontal
bars), `mode`, `direction`, `group_width` and `bar_gap` to
`Config.bar_groups`, and the series to `CategoryBars` series. The value and
category axis titles, the grid, the legend, the font, the text color, the
chart title, the annotation layer, the active layer, the selection and the
decoration positions also move to the Cartesian configuration. The category
axis of a horizontal document is inverted (`inverted`) so that the first
category is at the top as in the document, and `Y` axis annotations sit on
the same categories. The hover state, the background color, an explicit
`decorations.data_area` and value label positions are not moved. The
category axis line sits at 0 on the value axis (`AxisOptions::cross_at`).

From then on it is a Cartesian chart. `pick_chart_data` picks bars as
`PickedData::HistogramBin` (the category row is `bin_index`), and selecting
one with `PickedDataRef::HistogramBin` in `Config.picked_data` draws an
emphasis border inside the drawn shape. Selecting and moving the title,
legend, axes, tick labels and axis titles, moving and resizing the plot area,
the annotation layer, overlays and subplots, and PNG export work as on
Cartesian charts. In the browser, `FiggyChart.set_categorical_document(json)`
does the same.

```bash
cargo run -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo test -p figgy-renderer --features serde --test category_bars_render -- --test-threads=1
```

## 한국어 사용법

범주형 막대 차트는 직교 좌표 차트다. 범주형 축(`AxisOptions::categories`)이 있는 `Config`에
막대 시리즈마다 `DataRenderType::CategoryBars` 시리즈 하나를 두고, 직교 좌표 `Renderer`가 그린다.
위치와 값은 `ColumnSource`로 올린 컬럼에서 읽으며 스트리밍 소스는 거부한다.
`CategoricalChart`는 0.12까지의 릴리스가 쓴 문서 형식이며, `Config::from_categorical`이 이 문서를
범주형 막대 차트로 읽는다([직교 좌표 차트로 열기](#직교-좌표-차트로-열기)). 아래 규칙은 이 문서에 적용된다.
`CategoricalChart`는 이번 릴리스에서 문서 리더로만 남아 사용 중단(deprecated)되며, 다음 마일스톤에서 제거된다.

```rust,no_run
use renderer::{CategoricalChart, Category, BarSeries, Color, CategoryBarMode};
let chart = CategoricalChart {
    categories: vec![Category::new("a", "A"), Category::new("b", "B")],
    series: vec![
        BarSeries::new("before", "Before", vec![Some(12.0), Some(8.0)], Color::from_rgb8(70,130,205)),
        BarSeries::new("after", "After", vec![Some(18.0), Some(15.0)], Color::from_rgb8(255,166,105)),
    ],
    mode: CategoryBarMode::Grouped,
    ..Default::default()
};
chart.validate()?;
# Ok::<(), &'static str>(())
```

### 입력과 누적 규칙

- 범주와 시리즈는 각각 1개 이상이다. 범주 수, 시리즈 수와 `범주 수 × 시리즈 수`에는 상한이 없다.
- 범주·시리즈 ID는 각 목록에서 고유해야 한다. 이름은 중복될 수 있고, 이름을 바꿔도 ID는 바뀌지 않는다.
- 각 시리즈의 `values` 길이는 범주 수와 같아야 한다. `None`은 누락, `Some(0.0)`은 실제 0이다.
  둘 다 막대 면적은 없지만 0은 값·표기·선택 상태에 남는다. 포인터로는 면적이 있는 막대만 선택한다.
- `Grouped`는 나란히 그린다. 시리즈가 하나면 단일 막대다. 누락되어도 다른 막대의 자리는 이동하지 않는다.
- `Stacked`는 양수를 위쪽/오른쪽으로, 음수를 아래쪽/왼쪽으로 각각 0부터 쌓는다.
- `PercentStacked`는 범주별 양수 합계를 100%로 환산한다. 음수는 거절한다. 합계 0은 빈 막대이며
  퍼센트 표기는 `—`다. 원본 값은 바꾸지 않는다. 값이 아주 커도 합계가 넘치지 않는다.
- 일반 누적에서 합계가 유한한 숫자 범위를 벗어나면 거절한다. NaN·무한대는 모든 모드에서 거절한다.
- `Percent` 표기는 같은 범주의 시리즈 합계가 기준이다. 음수가 섞인 범주는 `—`로 표시한다.
- 값 축은 선형 축이다. `Chart::auto_fit_layer_union`이 0과 막대가 그리는 범위에 맞춘다.
  누적 막대는 값 컬럼마다 저장된 최솟값·최댓값으로 `[Σ min(0, 최솟값), Σ max(0, 최댓값)]`에
  맞춘다. 시리즈의 극값이 서로 다른 범주에 있으면 실제 누적보다 넓은 상한이 된다. 컬럼을 한 번에
  올리든 따로 올리든 결과는 같다. 100% 누적은 `0..100`에 맞춘다.
- 호스트가 직접 올린 컬럼도 같은 규칙을 따른다. `PercentStacked` 묶음의 값 컬럼에 음수가 있으면
  (값 컬럼의 최솟값이 0보다 작으면) 그리기와 선택 단계에서 해당 시리즈와 모드를 밝힌
  `InvalidSeriesConfig`로 거절한다. `Stacked`와 `Grouped`는 음수를 그린다.

선택과 개별 스타일은 ID를 참조하므로 이름을 바꿔도 같은 항목을 가리킨다. 항목을 삭제하면
해당 선택·개별 설정도 지워야 한다.

### 방향, 스타일과 개별 편집

`direction`은 `Vertical` / `Horizontal`이며 방향을 바꿔도 입력 값이나 범주 배열을 바꿀 필요가
없다. `value_title`과 `category_title`도 각 축의 역할을 따라 이동한다. 극좌표 차트(`Config::polar`,
범주형 각도 축)에서는 같은 시리즈가 로즈가 되어 범주의 각도 칸 안에서 반지름 방향으로 쐐기를
그리며 `Vertical`만 허용한다([POLAR.md](https://github.com/shim9610/figgy/blob/master/crates/renderer/POLAR.md#로즈극좌표-막대)).

스타일은 **차트 → 시리즈 → 개별 막대** 순서로 적용한다. `style: Some(...)`은 전체 스타일을
덮어쓰며 이후 상위 스타일 변경을 상속하지 않는다. `None`으로 돌리면 다시 상속한다.
색은 시리즈의 `color`, 개별 색은 `CategoryBarOverride.color`에서 정한다.

```rust,no_run
use renderer::{CategoricalChart, CategoryBarTarget, CategoryBarOverride,
    CategoryBarStyle, CategoryBarLabels, CategoryBarLabelFormat, Color, Material};
# fn edit(chart: &mut CategoricalChart) {
let target = CategoryBarTarget::new("a", "before");
let mut edit = CategoryBarOverride::new(target.clone());
edit.color = Some(Color::from_rgb8(30, 175, 165));
edit.style = Some(CategoryBarStyle {
    material: Material::SatinMetal,
    outline: true,
    ..chart.style.clone()
});
edit.labels = Some(CategoryBarLabels::Inside);
edit.label_format = Some(CategoryBarLabelFormat::ValuePercent);
// target당 개별 설정은 하나만 둘 수 있으므로 기존 설정을 지우고 새 설정을 넣는다.
chart.overrides.retain(|o| o.target != target);
chart.overrides.push(edit);
chart.selected = Some(target);
# }
```

| 옵션 | 기본값 | 범위·동작 |
|---|---|---|
| `material` | `Flat` | `Toon`·`Pearl`을 뺀 모든 [`Material`](RADIAL.md#materials). 막대는 평면 파이 윗면처럼 음영을 받는다 |
| `corner_radius` | 5 | 논리 픽셀 0~32. 막대 두께·길이의 절반 이내로 제한 |
| `outline` | false | 외곽선 켜기 |
| `outline_width` | 1 | 논리 픽셀 0~8. 도형 안쪽으로 그림 |
| `outline_color` | RGB(40,55,75) | RGBA 각 성분 0~1, 알파 허용 |
| `texture_strength` | 0.2 | 0~1, 0이면 무늬 끄기 |
| `texture_scale` | 1 | 0.1~8, 높일수록 무늬가 촘촘함 |
| `gloss` | 0.45 | 0~1. `SatinMetal`·`BrushedMetal`의 광택이며 `Enamel` 막대에서는 눈에 띄는 효과가 없다([재질](RADIAL.md#materials) 참고) |
| `emphasis_brightness` | 0.08 | 0~0.5, 호버·선택 시 밝기 증가 |
| `group_width` | 0.7 | 범주 칸에서 막대가 차지하는 비율, 0.1~0.95 |
| `bar_gap` | 3 | 묶음 내 막대 사이 논리 픽셀, 0~24 |
| `grid`, `legend` | true | 격자·시리즈 범례 표시 |

묶음 막대는 0 반대편 끝만 둥글게 만든다. 누적 막대는 양수·음수 스택 각각의 마지막
0이 아닌 조각 끝만 둥글게 만들고 내부 경계는 반듯하게 유지한다. 원본 값과 막대의 끝 위치는
재질·외곽선·호버에 따라 바뀌지 않는다. 질감에 외부 텍스처 파일은 필요 없다.
범주 색을 비교하기 위한 2D 표현이며 원형 차트의 입체 카메라·돌출 효과는 적용하지 않는다.

### 표기와 글꼴

- `labels`: `None` / `Inside` / `Outside` / `Auto`(기본).
  개별 `labels: None`은 상속, `Some(CategoryBarLabels::None)`은 숨기기다.
- `Auto`: 묶음은 바깥, 누적은 안쪽에 표시한다.
- `label_format`: `Value`(기본) / `Percent` / `ValuePercent` / `CategoryValue`.
  100% 누적도 값 라벨을 자동으로 퍼센트로 바꾸지 않는다. 원하는 표기를 명시한다.
- `label_decimals`: 0~6(기본 1), 불필요한 끝자리 0은 제거한다.
- `value_suffix`: 한 줄 24자까지. 공백을 포함해 `" kg"`처럼 지정한다.
- `font_size`: 논리 픽셀 8~32(기본 14), `font_family`: 기본 `sans-serif`.
- 범주·시리즈 이름과 ID는 제어 문자 없이 80자까지, 제목·축 제목은 160자까지다.

리더는 이 설정을 시리즈의 `labels`(`CategoryBarSeriesLabels`)와 `label_format`으로 옮기고,
`format_labels`가 막대마다 표시할 문자열을 `labels.text`에 기록한다. 렌더러는 값을 서식화하지 않고 이
문자열을 막대 끝에 그린다. 바깥 라벨은 막대 끝에서 6px 떨어진 곳에, 안쪽 라벨은 막대 가운데에 놓으며,
`Auto`는 묶음 막대는 바깥, 누적 막대는 들어가는 조각만 안쪽에 그린다. 어두운 막대 안쪽의 글자는 흰색,
그 밖에는 값 축 눈금 라벨의 글꼴·크기·색을 쓴다. 빈 문자열은 그리지 않고, 라벨끼리 밀어내지 않는다.
바깥 라벨이 데이터 영역 가장자리를 넘으면 막대 끝 안쪽에 그린다.
라벨은 `pick_chart_data`가 `PickedData::ValueLabel`로 고르며, 드래그하면 시리즈의 `labels.offsets`에
이동량이 기록된다. 원래 문서의 `DataLabel` 이동량도 `labels.offsets`로 옮긴다
([상호작용 가이드](INTERACTION.md)의 Value labels 참고).

### 직교 좌표 차트로 열기

```rust,no_run
use renderer::{CategoricalChart, Chart, ColumnSource, LayerRef, Renderer};
# fn open(r: &mut Renderer, json: &str) -> Result<(), Box<dyn std::error::Error>> {
let document: CategoricalChart = serde_json::from_str(json)?;
document.validate()?;
let (mut config, series, uploads) = renderer::config::Config::from_categorical(&document);
config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
    x: 0, y: 0, width: 900, height: 620,
});
// 위치 컬럼을 먼저, 값 컬럼은 한 번에 올린다. 업로드는 문서의 값을 빌려 쓴다.
let (positions, values) = uploads.split_first().expect("positions");
r.add_column(positions.column.clone(), &positions.values)?;
let columns: Vec<(&str, &dyn ColumnSource)> = values
    .iter()
    .map(|u| (u.column.as_str(), &u.values as &dyn ColumnSource))
    .collect();
r.add_columns(&columns)?;
let mut chart = Chart::new(config);
chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
# Ok(())
# }
```

`Config::from_categorical`은 범주를 방향에 맞는 범주형 축(세로 막대는 `bottom_x`, 가로 막대는
`left_y`)으로, `mode`·`direction`·`group_width`·`bar_gap`을 `Config.bar_groups`로, 시리즈를
`CategoryBars` 시리즈로 옮긴다. 값 축·범주 축 제목, 격자, 범례, 글꼴, 글자 색, 차트 제목, 주석 층,
활성 레이어, 선택, 장식 위치도 직교 좌표 설정으로 옮긴다. 가로 문서의 범주 축은 뒤집어
(`inverted`) 문서에서처럼 첫 범주가 맨 위에 오고, `Y` 축 주석도 같은 범주에 놓인다. 호버 상태, 배경색,
명시한 `decorations.data_area`, 값 라벨 위치는 옮기지 않는다. 범주 축의 축선은 값 축의 0에
놓인다(`AxisOptions::cross_at`).

그 뒤로는 직교 좌표 차트다. 막대는 `pick_chart_data`가 `PickedData::HistogramBin`(범주 행이
`bin_index`)으로 고르고, `Config.picked_data`의 `PickedDataRef::HistogramBin`으로 선택하면 그린
모양 안쪽에 강조 테두리를 그린다. 제목·범례·축·눈금 라벨·축 제목의 선택과 이동, 그리기 영역
이동·크기 조정, 주석 층, 오버레이와 서브플롯, PNG 내보내기는 직교 좌표 차트와 같다.
브라우저에서는 `FiggyChart.set_categorical_document(json)`가 같은 일을 한다.

```bash
cargo run -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo test -p figgy-renderer --features serde --test category_bars_render -- --test-threads=1
```
