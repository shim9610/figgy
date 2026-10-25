# Box plots / 박스플롯

A box plot is a Cartesian chart. Its boxes are `DataRenderType::BoxPlot`
series on a `Config` with a categorical axis (`AxisOptions::categories`), and
the Cartesian `Renderer` draws them: one box per category, with its median,
whiskers and caps, its mean and its outliers. The statistics are precomputed
by the host and uploaded through `ColumnSource`: a **record** of box
statistics per series and, when the series has outliers, a **ragged column**
of them. Streamed sources are rejected. `BoxPlotChart` is the document format
of the releases up to 0.12; `Config::from_boxplot` reads it as such a chart
([Opening a legacy document](#opening-a-legacy-document)). It is kept as a
deprecated document reader for this release and is removed in the next
milestone.

## Series and uploads

A `BoxPlot` series names three sources:

| Field | Holds |
|---|---|
| `x_column` | the category index column, `0, 1, …, n - 1` |
| `y_column` | the id of the series' record (`BOX_PLOT_RECORD`), one row per category |
| `outliers` | the id of a ragged column with one group of outliers per category, or `None` |

The record schema `BOX_PLOT_RECORD` has the fields `median`, `q1`, `q3`,
`whisker_low`, `whisker_high` (required) and `mean`, `sample_count`,
`ci_low`, `ci_high` (optional). A missing box is a row of NaN; a NaN optional
field leaves that mark out. Unlike a category bar, the roles do not swap with
the direction: the series names its record in the value column slot either
way.

```rust,no_run
use renderer::{
    BOX_PLOT_RECORD, Chart, Color, Column, ColumnSource, DataRenderType, LayerRef,
    RaggedSource, RecordSource, Renderer, SeriesConfig,
};
/// A host column with its range (NaN is a missing value).
fn column(data: Vec<f64>) -> Column<f64> {
    let finite = data.iter().copied().filter(|v| v.is_finite());
    let min = finite.clone().fold(f64::INFINITY, f64::min);
    let max = finite.fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}
# fn open(r: &mut Renderer, config: renderer::config::Config,
#         template: SeriesConfig) -> Result<(), Box<dyn std::error::Error>> {
// Two categories: `config.bottom_x.categories` holds them and
// `config.bar_groups` is grouped and vertical.
let nan = f64::NAN;
r.add_column("positions", &column(vec![0.0, 1.0]))?;
let stats = [
    column(vec![42.0, 55.0]), // median
    column(vec![30.0, 42.0]), // q1
    column(vec![55.0, 68.0]), // q3
    column(vec![12.0, 20.0]), // whisker_low
    column(vec![78.0, 85.0]), // whisker_high
    column(vec![44.0, nan]),  // mean
    column(vec![80.0, 64.0]), // sample_count
    column(vec![37.0, nan]),  // ci_low
    column(vec![47.0, nan]),  // ci_high
];
let fields: [(&str, &dyn ColumnSource); 9] =
    std::array::from_fn(|k| (BOX_PLOT_RECORD[k].0, &stats[k] as &dyn ColumnSource));
r.upsert_record(
    "response.record",
    &RecordSource { schema: BOX_PLOT_RECORD, fields: &fields },
)?;
// Category 0 has the outliers 90 and 94, category 1 none.
r.upsert_ragged(
    "response.outliers",
    &RaggedSource {
        values: &column(vec![90.0, 94.0]),
        offsets: &column(vec![0.0, 2.0, 2.0]),
    },
)?;
let series = [SeriesConfig {
    x_column: "positions".into(),
    y_column: "response.record".into(),
    render_type: DataRenderType::BoxPlot {
        color: Color::from_rgb8(65, 144, 208),
        style: Default::default(),
        labels: Default::default(),
        outliers: Some("response.outliers".into()),
        overrides: None,
    },
    ..template
}];
let mut chart = Chart::new(config);
chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.1)?;
# Ok(())
# }
```

`Renderer::upsert_record` and `upsert_ragged` replace a source of the same id
in place; `remove_record` and `remove_ragged` delete one. The record and the
ragged column live in the GPU column pool only; the CPU keeps their scalar
statistics (`record_stats`, `ragged_stats`) and nothing per box.

## Box groups and layout

The `BoxPlot` series of one layer form its box group. They stand side by side
in each category slot in series order, laid out by `Config.bar_groups`:

| `BarGroups` field | For box plots |
|---|---|
| `mode` | must be `Grouped`; stacking is not a box-plot operation |
| `direction` | `Vertical` (categories on `bottom_x`) or `Horizontal` (categories on `left_y`) |
| `group_width` | the slot share the group covers, 0.1–0.95 |
| `bar_gap` | the gap between the boxes of a group, logical px 0–24 |

`Config::validate_box_plots` (applied by the renderer) requires the
categorical axis of the direction and a linear value axis, and allows at most
16 box plot series per layer and 512 boxes. A layer draws either category bars
or box plots, not both. Horizontal box groups stand on the base panel only;
polar charts reject box plots.

## Statistics and clamping

The renderer does not sort samples, compute quartiles or infer the whisker
policy; whether the whiskers are the extremes, the last points inside 1.5 IQR
fences or something else is the host's choice. It also does not check the
order of a record it cannot read on the CPU. The drawing clamps instead:

- the body spans `min(q1, q3)` to `max(q1, q3)`;
- the median is clamped into the body;
- the low stem runs from `min(whisker_low, body low)` to the body, the high
  stem from the body to `max(whisker_high, body high)`;
- a notch is drawn only where both CI endpoints are finite;
- a row with a missing required field (or a missing position) draws no box,
  no mean and no outliers.

A `BoxPlotChart` document still validates the order (see
[the document rules](#document-rules)), so a document drawn through the
reader never needs the clamping.

## Styles and per-box overrides

`BoxPlotStyle` holds the look of a series' boxes:

| Option | Default / accepted values |
|---|---|
| `material` | Matte; any [`Material`](RADIAL.md#materials) except Toon and Pearl |
| `corner_radius` | 0 logical px, 0–3; capped by body size; notched silhouettes use the CI polygon |
| `outline`, `outline_width`, `outline_color` | true, 1.2 px (0–8), dark ink |
| `median_width`, `median_color` | 1.2 px (0–8), dark ink; zero hides the median |
| `whisker_width`, `whisker_color` | 1.2 px (0–8), dark ink; zero hides whiskers and caps |
| `caps`, `cap_ratio` | true, 0.5 (0–1 of box width) |
| `notched`, `notch_depth` | false, 0.22 (0–0.45 of box width per side) |
| `show_mean`, `mean_size`, `mean_color` | false, 7 px (1–24), red diamond |
| `show_outliers`, `outlier_size`, `outlier_color` | true, 7 px (1–24), dark ink |
| `outlier_shape`, `outlier_filled` | Circle (or Square), false; an unfilled marker is a 1.5 px ring |
| `texture_strength`, `texture_scale`, `gloss` | 0.2 (0–1), 1 (0.1–8), 0.45 (0–1) |
| `emphasis_brightness` | 0.08 (0–0.5), additive surface brightness on hover and selection |

All colors accept RGBA channels in 0–1. Materials affect only the box surface,
shaded like a flat pie top (see the [shared material table](RADIAL.md#materials));
whiskers, caps, the median, the mean and the outliers are flat. Turning the
outline off does not hide the median, the whiskers or the outliers.

`overrides` holds at most one `BoxPlotOverride` per box, keyed by
`BoxPlotTarget { category_id, series_id }`; its `color`, `style` and `labels`
replace the series' for that box only. A style override replaces the whole
style, so clone the series style first when changing one property, and clear
the `Option` to inherit again.

```rust,no_run
# use renderer::{BoxPlotOverride, BoxPlotStyle, BoxPlotTarget, Color, Material};
# fn edit(style: &BoxPlotStyle, overrides: &mut Vec<BoxPlotOverride>) {
let target = BoxPlotTarget::new("control", "response");
let mut individual = BoxPlotOverride::new(target.clone());
individual.color = Some(Color::from_rgb8(42, 168, 162));
individual.style = Some(BoxPlotStyle {
    material: Material::Matte,
    outline: false,
    ..style.clone()
});
// Replace an existing override with the same target instead of adding a duplicate.
overrides.retain(|o| o.target != target);
overrides.push(individual);
# }
```

## Value labels

`labels: BoxPlotSeriesLabels` holds the placement (`None`, `Median`,
`MedianAndCount`), the decimals (0–6), the suffix and `text`, the drawn string
of every box in category order. The renderer never writes `text`: the median
and the count are per-box data the configuration does not hold. A host that
uploads its own records writes it, for example with `format_box_plot_labels`
from the statistics it holds; `Config::from_boxplot` writes it from the
document. The renderer draws each string `10` px past the top of its box (the
highest of its high whisker, mean, notch and outliers), centred across the
box, in the value axis' tick label face, unless the placement (or the box's
override) is `None`; a label that would cross the data area's edge sits on
the other side of that top instead. An empty string draws nothing and labels
never displace each other. A label is a typed pick target (`PickedData::ValueLabel`), its
selection box is its frame, and a drag writes `labels.offsets` (logical
pixels, one per category); see Value labels in the
[interaction guide](INTERACTION.md).

## Value axis

`Chart::auto_fit_layer_union` fits the value axis to the pool's statistics of
every value field of the records (whiskers, quartiles, median, mean and CI;
not `sample_count`) and of the outlier columns, so toggling `show_mean` or
`show_outliers` does not rescale the chart. `auto_ticks` rounds it to nice
ticks. A fixed range is set on the value axis like any other Cartesian range;
the data and the pick are clipped to the plot.

## Picking, selection and interaction

`pick_chart_data` answers a box with `PickedData::HistogramBin`, its category
row as `bin_index`. Every drawn part answers its box: the body, the whiskers
and caps, the median, the mean and each outlier marker. Select a box through
`Config.picked_data` with `PickedDataRef::HistogramBin`; the selected box
draws an inside outline in the highlight colour around its painted shape
(the notch included) and gains its emphasis brightness. A host's hover and
click use the same pick (`pick_data` in the WASM wrapper); no statistic moves
on hover or selection. In the browser, `step_selection` walks the categories and the series of
the box group (the boxes, never their outliers), and `selection_details()`
reports the box's category and the statistics read back from its record.

The title, legend, axes, tick labels and axis titles are selected and dragged,
the plot area dragged and resized, the annotation layer drawn and edited,
overlays and subplots added and the chart exported to PNG as in every
Cartesian chart; see the
[interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md).
`AnnotationAxis::X` is the screen's horizontal axis: with vertical boxes, `X`
is the category index (the first category centred at 0) and `Y` the value.

## Opening a legacy document

```rust,no_run
use renderer::{
    BOX_PLOT_RECORD, Chart, ColumnSource, LayerRef, RaggedSource, RecordSource, Renderer,
    UploadValues, boxplot::BoxPlotChart,
};
# fn open(r: &mut Renderer, json: &str) -> Result<(), Box<dyn std::error::Error>> {
let document: BoxPlotChart = serde_json::from_str(json)?;
document.validate()?;
let (mut config, series, uploads) = renderer::config::Config::from_boxplot(&document);
config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
    x: 0, y: 0, width: 960, height: 640,
});
// The uploads borrow the document's summaries; nothing is copied.
for upload in &uploads {
    match &upload.values {
        UploadValues::BoxRecord(_) => {
            let fields = upload.values.box_fields().ok_or("record fields")?;
            let pairs: [(&str, &dyn ColumnSource); 9] = std::array::from_fn(|k| {
                (BOX_PLOT_RECORD[k].0, &fields[k] as &dyn ColumnSource)
            });
            r.upsert_record(
                upload.column.clone(),
                &RecordSource { schema: BOX_PLOT_RECORD, fields: &pairs },
            )?;
        }
        UploadValues::BoxOutliers(_) => {
            let (values, offsets) = upload.values.box_outliers().ok_or("outliers")?;
            r.upsert_ragged(
                upload.column.clone(),
                &RaggedSource { values: &values, offsets: &offsets },
            )?;
        }
        values => {
            r.add_column(upload.column.clone(), values)?;
        }
    }
}
let mut chart = Chart::new(config);
if document.value_range.is_none() {
    chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.1)?;
}
# Ok(())
# }
```

`Config::from_boxplot` moves the categories to the categorical axis of the
direction (`bottom_x` for vertical boxes, `left_y` for horizontal ones,
top-down as before), `direction`, `group_width` and `box_gap` to a grouped
`Config.bar_groups`, and each document series to a `BoxPlot` series reading
`boxplot.positions`, its record `boxplot.record.<series id>` and, when any of
its boxes has an outlier, `boxplot.outliers.<series id>`. The series takes its
colour, its style (or the chart's), the chart's label settings with the text
written from the medians and counts, and its overrides. `value_range` becomes
the fixed value axis range; the titles, grid, legend, font, text colour, chart
title, annotations, active layer, selection and decoration offsets map onto
their Cartesian options. The hover state, the background colour (the host's
clear colour draws) and an explicit `decorations.data_area` are not carried;
the data label offsets become the series' `labels.offsets`. In the browser, `FiggyChart.set_boxplot_document(json)`
does the same.

### Document rules

`BoxSummary::new(low, q1, median, q3, high)` receives the **actual whisker
endpoints**. `mean`, `sample_count`, `median_ci` and `outliers` are optional.
`BoxPlotChart::validate` requires `whisker_low <= q1 <= median <= q3 <=
whisker_high`, all finite; outliers finite and strictly outside the whiskers;
a CI that contains the median (it may extend beyond the body); an explicit CI
for every box of a notched series; positive counts. `None` is a missing box
that leaves its group slot empty. Limits: 1–64 categories, 1–16 series, at
most 512 boxes including missing ones, at most 128 outliers per box and 4096
in all.

## Examples and tests

```bash
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
cargo test --locked -p figgy-model --features serde box_plot
cargo test --locked -p figgy-renderer --features serde --test box_plot_render -- --test-threads=1
```

The gallery writes five 960 × 640 PNGs (matte, satin, horizontal, grouped
notch and mean with a selected box) from synthetic documents opened through
the reader. The playground's Cartesian `BoxPlot` scene edits the series
style. The browser test `crates/web/tests/boxplot_document_browser.rs` opens a
document in `FiggyChart`.

## 한국어 사용법

박스플롯은 직교 좌표 차트다. 범주형 축(`AxisOptions::categories`)이 있는 `Config`에 시리즈마다
`DataRenderType::BoxPlot` 시리즈 하나를 두고, 직교 좌표 `Renderer`가 그린다. 시리즈의 `x_column`은
범주 인덱스(`0..n-1`) 컬럼, `y_column`은 박스 통계 레코드(`BOX_PLOT_RECORD`)의 ID, `outliers`는
범주마다 이상치 묶음 하나를 담은 가변 길이 컬럼의 ID다. 레코드와 가변 길이 컬럼은
`Renderer::upsert_record`, `upsert_ragged`로 올리며 스트리밍 소스는 거부한다.
`BoxPlotChart`는 0.12까지의 문서 형식이고, `Config::from_boxplot`이 이 문서를 박스플롯 차트로 읽는다.
`BoxPlotChart`는 이번 릴리스에서 문서 리더로만 남아 사용 중단(deprecated)되며, 다음 마일스톤에서 제거된다.

박스플롯은 집계한 통계값을 받는다. 문서에서는 `BoxSummary::new(수염 하한, Q1, 중앙값, Q3,
수염 상한)`으로 만들고, 평균·표본 수·이상치·중앙값 신뢰구간은 필요할 때 추가한다.
수염이 최솟값/최댓값인지, 1.5 IQR 안의 관측값인지 등은 호스트에서 결정한다.
렌더러는 원본 표본으로 통계를 계산하거나 수염 규칙을 추측하지 않는다. 레코드의 순서도 검사하지 않고
그릴 때 맞춘다. 상자는 Q1·Q3 중 작은 값부터 큰 값까지, 중앙값은 상자 안으로, 수염은 상자 바깥으로
그리며 신뢰구간 양 끝이 모두 있을 때만 노치를 그린다. 필수 값이 없는 행은 그리지 않는다.
문서의 `validate`는 예전처럼 순서를 검사한다.

노치를 켜려면 `median_ci`를 넣어야 한다. 신뢰구간이 상자 밖으로 나와도 잘라서
통계적 의미를 바꾸지 않는다. Q1과 Q3가 같으면 채운 상자는 없지만 중앙값과 수염 등은
설정에 따라 표시한다. `None`은 누락값이며, 그 범주·시리즈의 자리는 그대로 둔다.

가로형은 `Config.bar_groups`의 `direction = Horizontal`, 그룹형은 한 레이어에 박스플롯 시리즈를 여러 개
두면 된다. `bar_groups`의 `mode`는 `Grouped`여야 한다. 각 상자의 색·재질·외곽선·노치·표기는
`BoxPlotOverride`로 바꾼다. 스타일은 전체 교체이므로 부모 스타일을 복제한 뒤 필요한 값만 바꾸면 된다.
개별 설정을 `None`으로 되돌리면 상위 설정을 따른다. 외곽선을 꺼도 중앙값과 수염은 별도 설정을 유지한다.

색이나 호버 강조를 바꿔도 통계값의 좌표는 움직이지 않는다. `Chart::auto_fit_layer_union`은 숨긴
평균·이상치·신뢰구간까지 포함해 값 축을 맞추므로 표시 옵션을 켜고 끄는 것만으로 축이 바뀌지 않는다.
직접 범위를 지정하면 영역 밖의 도형과 선택 판정을 함께 자른다.

상자는 `pick_chart_data`가 `PickedData::HistogramBin`(범주 행이 `bin_index`)으로 고른다. 상자 몸통,
수염, 캡, 중앙값, 평균, 이상치 모두 자기 상자를 가리킨다. `Config.picked_data`의
`PickedDataRef::HistogramBin`으로 선택하면 그린 모양 안쪽에 강조 테두리를 그린다. 값 라벨 문자열은
`labels.text`에 두며, 렌더러는 상자 꼭대기(높은 수염·평균·노치·이상치 중 가장 높은 곳)에서 10px 위에
그린다. 데이터 영역 가장자리를 넘으면 꼭대기 아래에 그린다. 라벨은 `PickedData::ValueLabel`로 고르고 드래그하면
`labels.offsets`에 기록된다.
입력 상한은 64범주·16시리즈·총 512항목이며, 문서는 상자 하나에 이상치를 최대 128개, 모두 4096개 받는다.

## 제목·축·라벨·범례 선택과 이동

제목·범례·축·눈금 라벨·축 제목의 선택과 이동, 그리기 영역 이동·크기 조정, 오버레이와 서브플롯,
PNG 내보내기는 직교 좌표 차트와 같다. 좌표 단위, 요소별 이동 규칙, 캐시·저장 복원 및 웹 연결은
[공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)를 참고한다.
브라우저에서는 `FiggyChart.set_boxplot_document(json)`로 문서를 연다.

### 주석 층

주석 층은 직교 좌표 차트의 주석 층이다. `AnnotationAxis::X`는 화면 가로, `Y`는 화면 세로다.
세로 상자이면 `X`가 범주 인덱스(첫 범주 중심이 0), `Y`가 값 축이다. 자세한 계약은
[공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer)를 참고한다.
