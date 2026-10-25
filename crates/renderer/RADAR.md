# Radar charts

A radar (spider) chart is a polar chart. Its categories are the angle axis:
`bottom_x.categories` (`AxisOptions::categories`) on a `Config` with
`polar: Some(..)`, so category `i` has a spoke at `i / n` of the turn. Its
polygons are `DataRenderType::Radar` series, one per compared series, and the
Cartesian `Renderer` draws them through the polar line, area and scatter
paths: a closed outline, a fill to the centre and a marker at every vertex.
The values are the host's and enter through `ColumnSource`, one column per
series; streamed sources are rejected. `RadarChart` is the document format of
the releases up to 0.12; `Config::from_radar` reads it as such a chart
([Opening a legacy document](#opening-a-legacy-document)). It is kept as a
deprecated document reader for this release and is removed in the next
milestone.

## Series and uploads

A `Radar` series reads two columns, each with one value per category:

| Field | Holds |
|---|---|
| `x_column` | the category index column, `0, 1, …, n - 1` |
| `y_column` | the value of each category on the radius axis (`left_y`); NaN for a missing value |

```rust,no_run
use renderer::config::Category;
use renderer::polar::{GridShape, PolarOptions};
use renderer::{
    Chart, Color, Column, DataRenderType, LayerRef, RadarSeriesStyle, Renderer, SeriesConfig,
};
/// A host column with its range (NaN is a missing value).
fn column(data: Vec<f64>) -> Column<f64> {
    let finite = data.iter().copied().filter(|v| v.is_finite());
    let min = finite.clone().fold(f64::INFINITY, f64::min);
    let max = finite.fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}
# fn draw(r: &mut Renderer) -> Result<(), Box<dyn std::error::Error>> {
let mut config = renderer::default::default_config();
// Five spokes, the first one up and the others clockwise, joined by
// polygon rings.
config.bottom_x.categories = Some(
    ["Speed", "Safety", "Size", "Docs", "Tests"]
        .iter()
        .map(|c| Category::new(c.to_lowercase(), *c))
        .collect(),
);
config.polar = Some(PolarOptions {
    zero_angle_degrees: 90.0,
    clockwise: true,
    grid_shape: Some(GridShape::Polygon),
    ..PolarOptions::default()
});
r.add_column("categories", &column(vec![0.0, 1.0, 2.0, 3.0, 4.0]))?;
r.add_column("release1", &column(vec![6.0, 8.0, 4.0, 5.0, 7.0]))?;
let series = vec![SeriesConfig {
    series_id: "release1".into(),
    source_id: None,
    label: None,
    x_column: "categories".into(),
    y_column: "release1".into(),
    render_type: DataRenderType::Radar {
        color: Color::from_rgb8(60, 110, 200),
        style: RadarSeriesStyle::default(),
        labels: Default::default(),
    },
}];
// The radius axis fits the values' statistics; the angle axis keeps its
// categories.
let mut chart = Chart::new(config);
chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
let png = r.export_panel_png_bytes(&chart, &series, 2.0)?;
# let _ = png;
# Ok(())
# }
```

A radar series is held to these rules (`Config::validate_radar` and
`data_config::validate_radar_series`, checked when the series are set):

- It is drawn on a polar chart, the parent's or a subplot's own, whose angle
  axis is categorical (1 to 64 categories), with chord segments
  (`PolarInterpolation::Chord`): a polygon's edges are straight.
- At most 16 radar series share one chart.
- Its colour has channels in `0..=1`, its style passes
  `RadarSeriesStyle::validate`, and `labels.text` is empty or one one-line
  string of at most 160 characters per category.
- Both columns hold exactly one value per category. The lengths are data, so
  the renderer checks them against the pool when it prepares the polygon.
- It draws in the precise style only: `draw_style_supports` answers `false`
  for a stylized draw style, which draws no radar series.

## The polar chart

The angle axis of a radar chart is categorical: category `i` sits at `i / n`
of the full turn from `PolarOptions::zero_angle_degrees`, growing in the
`clockwise` direction, or at `i / n` of the `sector` from its start
(`PolarOptions::angle_map`). Its spokes are the angle axis' major grid at the
categories (`grid.show_major_x`), its labels are `categories[i].label` placed
outside the ring along each spoke, thinned by the stride rule of the other
axes when they crowd, and it has no minor ticks. The rings are the radius
axis' major ticks (`grid.show_major_y`), and the outer ring is the angle axis
line. `PolarOptions::grid_shape` draws them as circles (`None` or
`Circle`, the polar look) or as polygons joined at the category angles
(`Polygon`, the radar look; it needs a categorical angle axis). The radius
axis stays numeric: the ray at `radial_label_angle_degrees` carries its
labels.

The value range is the radius axis' (`left_y.min` to `left_y.max`), shared by
every spoke; per-spoke scaling is the host's job. A value below the range
start has no place on the plot and breaks the polygon there, as it breaks a
polar line; a value past the end is clipped at the outer ring.

## Look

`RadarSeriesStyle` sets the look of one series. Every field is optional and
an unset field takes its default:

| Field | Default | Meaning |
|---|---|---|
| `line_width_px` | 2 | outline width in logical pixels, `0..=12`; zero draws no outline |
| `fill` | the colour at `fill_opacity` | the fill colour; a transparent colour draws the outline only |
| `fill_opacity` | 0.25 | the alpha factor of the series colour when `fill` is unset |
| `marker` | `Circle` | `Circle`, `Square` or `None` at every vertex |
| `marker_size_px` | 7 | the circle's diameter or the square's side, `1..=24` |

The outline is the series colour, a solid line through the vertices in
category order with one more segment from the last vertex back to the first.
The fill reaches from the polygon to the centre, or to the inner ring with an
`inner_radius_fraction`. In a sector the closing trapezoid is not filled (it
would cover the others): the fill is the region between the open outline and
the inner ring there, while the outline still closes. A NaN vertex breaks the
outline and the fill on both of its sides. Every series' fill is drawn before
any series' outline and markers, in series order.

## Picking and selection

The vertices, the outline (its closing segment included) and the fill pick
through the shared polar picks of lines, fills and markers: `pick_data`
answers `PickedData::Point { point_index, .. }` with the category index of the
vertex under the pointer, the nearer endpoint of the edge or the fill under
it. A marker wins a tie over the outline, and the outline over the fill. A
selected vertex (`Config.picked_data` with a `PickedDataRef::Point`) takes the
chart's picked ring, as a polar line or marker does. In the browser,
`FiggyChart.step_selection(dx, dy)` steps a selected vertex along the
categories with `dx` and across the radar series with `dy`, and
`selection_details` reports the category of a selected vertex with its value.

## Value labels

`labels: RadarSeriesLabels` holds the placement (`RadarLabels::None`,
`Values` or `Emphasized`), the decimals, the suffix and `text`, one drawn
string per category. The renderer never formats a value: `text` is written
at edit time by `data_config::format_radar_labels` or by a document reader.
The renderer draws each string outside its vertex along the spoke, `9` px
from it, as the former radar chart placed it, in the radius axis' tick label
face: always with `Values`, and with `Emphasized` only while the series has a
selected vertex or label. An empty string draws nothing, and labels never
displace each other. A label that would cross the data area's edge outward
goes inward along its spoke instead. A label is a typed pick target
(`PickedData::ValueLabel`), its selection box is its frame, and a drag writes
`labels.offsets` (logical pixels, one per category); see Value labels in the
[interaction guide](INTERACTION.md).

## Decorations and annotations

The title, the legend, the category labels (the angle axis' tick labels,
`Element::AxisLabel(Side::Bottom)`, moved along their spokes), the ring
labels and the ray (`Element::AxisLabel(Side::Left)` and
`Element::Axis(Side::Left)`, which turns around the centre) and the data area
are the polar chart's decorations, with its hit regions, drags and resize
handles; the
[interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)
has the rules. The annotation layer draws ratio anchors relative to the data
area; a data anchor has no place on polar axes and is skipped with a render
warning.

## Opening a legacy document

```rust,no_run
# #![allow(deprecated)]
use renderer::radar::RadarChart;
use renderer::{Chart, LayerRef, Renderer, config::Config};
# fn open(r: &mut Renderer, document: &RadarChart) -> Result<(), Box<dyn std::error::Error>> {
document.validate()?;
let (config, series, uploads) = Config::from_radar(document);
for upload in &uploads {
    // The values upload straight from the document, through a borrowed
    // adapter.
    r.add_column(upload.column.clone(), &upload.values)?;
}
let mut chart = Chart::new(config);
if document.value_range.is_none() {
    chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
}
# let _ = chart;
# Ok(())
# }
```

`Config::from_radar(&RadarChart)` returns the polar `Config`, one `Radar`
series per document series in document order and the uploads: the category
indices (`radar.positions`) first, then one value column per series
(`radar.values.<series id>`). The field map:

- `categories` are the angle axis' categories; `style.start_angle_degrees`
  (screen degrees with y down) is `zero_angle_degrees` (with y up, so
  negated) and the ring labels' ray; `style.clockwise` is `clockwise`;
  `style.grid_shape` is the polar `grid_shape`.
- `value_range` is the radius axis range with `ring_spacing` as its spacing
  (nice spacing without one). Without a `value_range` the range is the
  host's to fit, as the former chart fitted the values; the browser wrapper
  and the gallery fit it from zero when no value is negative.
- `grid` shows the rings (in `style.ring_color`, the outer ring too) and the
  spokes (in `style.spoke_color`).
- Each series takes its colour and its style with every unset field read
  from the chart style (`RadarSeriesStyle::resolved_from`); the chart's
  `labels`, `label_decimals` and `value_suffix` go to every series, with the
  text `format_radar_labels` writes from the values.
- `font_family`, `font_size` and `text_color` are the category and ring
  labels and the legend; `legend` is the legend's visibility with one entry
  per series; the resolved title is `chart_title`.
- A selected vertex is the typed selection; the offsets of the title, the
  legend and the ring labels are the matching offsets; `active_layer` and
  `annotations` carry over.
- Not carried: `hovered` (transient), a selected whole series (a selection
  names a vertex), `style.emphasis_brightness` (the former renderer
  brightened the hovered or selected series; a picked vertex takes the
  picked ring instead), `background` (the host's clear colour), an explicit
  `decorations.data_area`, the spoke label offsets and the decoration
  selection. The data label offsets become the series' `labels.offsets`.

`FiggyChart.set_radar_document(json)` opens a document in the browser;
`FiggyBoundedChart`'s `"radar"` family is an error naming it for one release.

## Examples and tests

```bash
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo test --locked -p figgy-renderer --features serde --test radar_series_render -- --test-threads=1
```

## 한국어 사용법

레이더(거미줄) 차트는 극좌표 차트다. 범주가 각도 축이 되어, `polar: Some(..)`인 `Config`의
`bottom_x.categories`(`AxisOptions::categories`)에 범주를 넣으면 범주 `i`의 스포크가 한 바퀴의
`i / n` 위치에 놓인다. 비교할 시리즈마다 `DataRenderType::Radar` 시리즈를 하나씩 두며, 직교 차트
`Renderer`가 극좌표 선·영역·산점도 경로로 닫힌 외곽선, 중심까지의 채움, 꼭짓점 마커를 그린다.
값은 호스트가 시리즈마다 열 하나로 `ColumnSource`를 거쳐 올린다. 스트리밍 소스는 거부한다.
0.12까지의 문서 형식인 `RadarChart`는 `Config::from_radar`로 이 차트로 읽는다.
`RadarChart`는 이번 릴리스에서 문서 리더로만 남아 사용 중단(deprecated)되며, 다음 마일스톤에서 제거된다.

`Radar` 시리즈의 `x_column`은 범주 번호 열(`0, 1, …, n - 1`), `y_column`은 범주별 반지름 축
(`left_y`) 값이며 빠진 값은 NaN으로 둔다. 두 열 모두 범주 수만큼 값이 있어야 하고, 렌더러가
준비 단계에서 풀의 길이로 확인한다. 레이더 시리즈는 각도 축이 범주형(1~64개)이고 보간이
`PolarInterpolation::Chord`인 극좌표 차트(부모 차트나 서브플롯 자신의 차트)에서만 그리며, 한
차트에 16개까지 둘 수 있다. 정밀(precise) 스타일에서만 그리고, 스타일 모드에서는
`draw_style_supports`가 `false`를 돌려준다.

범주 `i`는 `PolarOptions::zero_angle_degrees`에서 `clockwise` 방향으로 한 바퀴의 `i / n`, 또는
`sector`가 있으면 섹터 시작에서 섹터의 `i / n`에 놓인다(`PolarOptions::angle_map`). 스포크는
범주 위치의 각도 축 주 격자(`grid.show_major_x`), 라벨은 스포크 끝 바깥의
`categories[i].label`이며 보조 눈금은 없다. 고리는 반지름 축 주 눈금(`grid.show_major_y`),
바깥 고리는 각도 축 선이다. `PolarOptions::grid_shape`가 `None`이나 `Circle`이면 고리를 원으로,
`Polygon`이면 범주 각도에서 꺾이는 다각형으로 그린다(`Polygon`은 범주형 각도 축이 있어야 한다).
반지름 축은 숫자 축으로 남아 `radial_label_angle_degrees` 방향의 선에 라벨을 단다. 값 범위는
반지름 축 범위이며 모든 스포크가 공유한다. 스포크별 정규화는 호스트가 맡는다. 범위 시작보다
작은 값은 다각형을 그 자리에서 끊고, 끝보다 큰 값은 바깥 고리에서 잘린다.

`RadarSeriesStyle`의 필드는 모두 선택이며 비우면 기본값을 쓴다. 외곽선 굵기
`line_width_px`(2, 0이면 외곽선 없음), 채움 색 `fill`(없으면 시리즈 색에 `fill_opacity`를 곱한
알파, 투명색이면 외곽선만), `fill_opacity`(0.25), 마커 `marker`(`Circle`, `Square`, `None`),
마커 지름·변 길이 `marker_size_px`(7)가 있다. 외곽선은 범주 순서로 꼭짓점을
잇고 마지막 꼭짓점에서 첫 꼭짓점으로 한 구간을 더 그린다. 채움은 다각형에서 중심까지,
`inner_radius_fraction`이 있으면 안쪽 고리까지 칠한다. 섹터에서는 닫는 구간의 사다리꼴을
칠하지 않으므로, 외곽선은 닫히지만 채움은 열린 외곽선과 안쪽 고리 사이 영역이다. NaN 꼭짓점은
양쪽의 외곽선과 채움을 끊는다.

꼭짓점, 외곽선(닫는 구간 포함), 채움은 극좌표 선·채움·마커의 공통 픽으로 고른다. `pick_data`는
포인터 아래 꼭짓점, 또는 아래 변이나 채움에서 가까운 끝점의 범주 번호를
`PickedData::Point { point_index, .. }`로 돌려준다. 겹치면 마커가 외곽선을, 외곽선이 채움을
이긴다. `Config.picked_data`의 `PickedDataRef::Point`로 선택한 꼭짓점에는 극좌표 선이나 마커처럼
차트의 선택 고리가 그려진다. 브라우저의 `FiggyChart.step_selection(dx, dy)`는 `dx`로 범주를
따라, `dy`로 레이더 시리즈를 건너 꼭짓점을 옮기고, `selection_details`는 선택한 꼭짓점의 범주와
값을 알려 준다.

`labels: RadarSeriesLabels`는 위치(`RadarLabels::None`, `Values`, `Emphasized`), 소수 자릿수,
접미사, 범주마다 하나인 문자열 `text`를 담는다. 렌더러는 값을 포맷하지 않으며, `text`는 편집할
때 `data_config::format_radar_labels`나 문서 판독기가 적는다. 렌더러는 각 문자열을 꼭짓점에서
바큇살 바깥쪽으로 9px 떨어진 곳에 반지름 축 눈금 라벨 글꼴로 그린다. `Values`는 항상,
`Emphasized`는 시리즈에 선택한 꼭짓점이나 라벨이 있을 때만 그린다. 빈 문자열은 그리지 않고
라벨끼리 밀어내지 않는다. 바깥쪽 라벨이 데이터 영역 가장자리를 넘으면 바큇살 안쪽에 그린다. 라벨은 `PickedData::ValueLabel`로 고를 수 있고, 선택 상자는 라벨의
테두리이며, 끌면 `labels.offsets`(범주마다 하나, 논리 픽셀)에 기록한다.

제목, 범례, 범주 라벨(각도 축 눈금 라벨, 스포크를 따라 이동), 고리 라벨과 반지름 선(중심을
돌며 회전), 그리기 영역은 극좌표 차트의 장식 요소이며 선택·이동·크기 조절 규칙은
[공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)를
따른다. 주석 층은 데이터 영역 기준 비율 앵커를 그리고, 데이터 앵커는 극좌표 축에 자리가 없어
렌더 경고와 함께 건너뛴다.

### 이전 문서 열기

`Config::from_radar(&RadarChart)`는 극좌표 `Config`, 문서 시리즈마다 하나인 `Radar` 시리즈,
업로드 목록을 돌려준다. 업로드는 범주 번호 열(`radar.positions`)이 먼저이고 그다음 시리즈마다
값 열(`radar.values.<시리즈 id>`)이 하나씩이며, 문서의 값을 빌려 그대로 올리므로 복사하지
않는다. `style.start_angle_degrees`(y가 아래로 커지는 화면 각도)는 부호를 바꿔
`zero_angle_degrees`와 고리 라벨 선의 각도가 되고, `clockwise`와 `grid_shape`는 극좌표 옵션으로
옮긴다. `value_range`는 반지름 축 범위, `ring_spacing`은 눈금 간격이 된다. `value_range`가
없으면 범위는 호스트가 맞추며, 브라우저 래퍼와 갤러리는 음수 값이 없으면 0부터 맞춘다. `grid`는
고리(`style.ring_color`, 바깥 고리 포함)와 스포크(`style.spoke_color`)를 보인다. 시리즈 스타일의
빈 필드는 차트 스타일로 채우고(`RadarSeriesStyle::resolved_from`), 차트의 라벨 설정과 값에서
적은 라벨 문자열을 모든 시리즈에 넣는다. 글꼴과 글자색은 범주·고리 라벨과 범례로, 선택한
꼭짓점은 선택 상태로, 제목·범례·고리 라벨의 이동량은 해당 오프셋으로 옮긴다. 호버, 시리즈 전체
선택, 차트 스타일의 `emphasis_brightness`(이전 렌더러가 강조한 시리즈를 밝히던 값으로, 이제 고른
꼭짓점은 선택 고리로 강조한다), 배경, 명시한 그리기 영역, 스포크 라벨 이동량은 옮기지 않는다. 데이터 라벨 이동량은 시리즈의 `labels.offsets`가 된다. 브라우저에서는
`FiggyChart.set_radar_document(json)`로 문서를 열며, `FiggyBoundedChart`의 `"radar"` 계열은 이 메서드를
안내하는 오류를 낸다.
