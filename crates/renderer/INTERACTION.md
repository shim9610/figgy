# 새 차트의 선택·이동·크기 조정

원형·범주형 막대·박스플롯은 데이터 모델이 서로 다르지만, 영역·제목·축·라벨·범례는
기존 `HitMap → Selectable → Draggable / Resizable` 계약을 사용한다. 포인터 이벤트 전달과
드래그 중 프레임 요청은 호스트가 담당한다. 클릭 영역이나 라벨 위치를 호스트에서 다시 계산하지 않는다.

## 데이터 선택과 장식 요소 선택

- `frame.hit_test([x, y])`는 기존대로 조각·막대·박스의 **데이터**를 고른다.
- `frame.decorations().hit_test([x, y])`는 제목·축·라벨·범례를 고른다.
- 선택한 영역의 리사이즈 핸들을 먼저 검사한다. 핸들은 영역 경계 밖으로도 걸쳐 있다.
- 그다음 제목·라벨·축 같은 앞쪽 요소, 데이터, `DataArea` 순서로 판정한다. 영역이 데이터
  클릭을 가로채면 안 된다. 영역 편집 모드를 켰을 때만 영역을 데이터보다 우선한다.
- 좌표와 이동량은 차트 내부의 **논리 픽셀**이다. CSS로 확대된 캔버스라면 화면 좌표를
  차트의 논리 크기로 변환한다. DPR을 곱한 framebuffer 좌표를 넘기지 않는다.

장식 요소 선택은 `chart.decorations.selected`, 데이터 선택은 기존 `selected` 또는
`interaction.selected`에 저장한다. 별개 상태이므로 호스트가 한 종류만 선택하려면
다른 쪽 선택을 명시적으로 지운다. 원형 호버의 `interaction.hovered`도 그대로 사용한다.

| 요소 | `DecorationId` | 이동 규칙 |
|---|---|---|
| 그리기 영역 | `DataArea` | 영역 이동과 8개 핸들 크기 조정. 도형·축·라벨을 다시 배치 |
| 차트 제목 | `ChartTitle` | 화면 방향으로 자유 이동 |
| 축 선 | `Axis(Side)` | 축에 수직인 방향만 이동. 데이터·그리드는 고정, 눈금 글자는 축을 따라 이동 |
| 눈금·범주 라벨 | `AxisLabel(Side)` | 해당 축의 라벨 묶음을 이동 |
| 축 제목 | `AxisTitle(Side)` | 회전된 제목도 포인터의 화면 방향으로 이동 |
| 범례 | `Legend` | 기호와 모든 행을 함께 이동 |
| 원형 조각 라벨 | `RadialLabel(RadialTarget)` | 이름·수치 줄을 함께 이동. 외부 라벨 연결선 끝도 갱신 |
| 막대·박스 값 라벨 | `DataLabel { category_id, series_id }` | 해당 데이터 라벨만 이동. 재정렬해도 같은 ID에 적용 |

원형에는 축과 범례를 그리지 않으므로 이들의 히트 영역도 없다. 숨긴 제목·라벨·범례는
히트맵에 등록되지 않는다. 축 라벨은 기존 Cartesian 계약처럼 묶음 선택이며, 범주 이름 하나마다
독립적으로 이동하는 계약은 아니다. 영역 조정은 데이터 값이나 축 범위를 바꾸지 않는다.
데이터 값 자체를 드래그로 바꾸는 기능과도 별개다.

## Manual label placement

Use the displayed frame's decoration hit map to select and drag labels. Each
radial label and each bar/box value label can move independently; axis tick labels
move together as a group. Drag deltas are floating-point logical pixels, so small
fractional adjustments are supported. Store selection in `decorations.selected`,
apply `drag_by` to `chart.decorations`, and prepare a new frame before drawing or
using the updated hit map. Export uses the same stored offsets.

Reset an element with `decorations.reset(&id)` to restore its automatic position.

## 호출 순서

```rust,no_run
use renderer::{RadialChart, RadialRenderer, DecorationId};
# fn edit(renderer: &mut RadialRenderer, chart: &mut RadialChart) -> Result<(), renderer::RadialError> {
// 실제 모델/크기/DPR에 맞춰 준비한 프레임으로만 선택한다.
let frame = renderer.prepare(chart, (800, 520), 2.0)?;
let map = frame.decorations();
if let Some(hit) = map.hit_test([120.0, 180.0]) {
    let target: DecorationId = map.element(hit).unwrap().id.clone();
    chart.decorations.selected = Some(target);

    // 프레임별 포인터 이동량. 누적 drag_delta를 매번 더하지 않는다.
    let result = map.drag_by(&mut chart.decorations, hit, 8.0, -4.0);
    // result: Some(Moved), Some(Rejected(OutOfBounds)), 또는 없는 hit면 None.
    // 거절 시 상태는 바뀌지 않는다.
    let _ = result;
}
let next_frame = renderer.prepare(chart, (800, 520), 2.0)?;
// next_frame.draw(...) 또는 renderer.export_rgba_async(...).
# Ok(()) }
```

기존 트레잇을 직접 사용하는 경로도 같다.

```rust,no_run
# fn drag(map: &renderer::decoration::DecorationMap, hit: renderer::HitId,
#         chart: &mut renderer::RadialChart) {
let element = map.hit_map().get(hit).unwrap();
let result = element.as_draggable().unwrap()
    .drag_by(&mut chart.decorations, 8.0, -4.0);
# let _ = result;
# }
```

`HitId`는 해당 프레임 히트맵에서만 유효하다. 장기간 저장할 때는 `DecorationId`를 쓴다.
크기·폰트·데이터·방향·모양을 바꾼 다음에는 `prepare`한 새 프레임에서 다시 조회한다.
`map.bounds(hit)`와 `map.selection_box(hit)`도 논리 픽셀이다. 선택 테두리는 공통
`SelectionBox` 규칙으로 렌더러가 그린다.

## 그리기 영역 선택과 크기 조정

`DecorationId::DataArea`의 히트맵 요소는 `as_draggable()`와 `as_resizable()`을 모두 제공한다.
선택 테두리의 8개 핸들, 핸들 판정과 이동 방향은 기존 공통 리사이즈 계약을 사용한다.
`DecorationMap::hit_resize_handle`과 `resize_by`는 같은 트레잇 호출을 편하게 감싼 메서드다.

```rust,no_run
# fn resize(renderer: &mut renderer::RadialRenderer, chart: &mut renderer::RadialChart)
# -> Result<(), renderer::RadialError> {
use renderer::{DecorationId, layout::NudgeResult, resize::ResizeHandle};
let displayed = renderer.prepare(chart, (800, 520), 2.0)?;
let map = displayed.decorations();
let (id, _) = map.entries().find(|(_, e)| e.id == DecorationId::DataArea).unwrap();
let mut trial = chart.clone();
trial.decorations.selected = Some(DecorationId::DataArea);
if map.resize_by(&mut trial.decorations, id, ResizeHandle::SE, -80., -60.)
    == Some(NudgeResult::Moved)
{
    // 글자가 들어갈 공간까지 검사한 뒤 편집 결과를 확정한다.
    let next = renderer.prepare(&trial, (800, 520), 2.0)?;
    *chart = trial;
    // next.draw(...)
    let _ = next;
}
# Ok(()) }
```

`decorations.data_area: Option<RectF>`는 그리기 영역을 논리 픽셀로 보관한다. `None`은 글자와
범례에 필요한 공간을 측정해 자동 배치한다. 지정한 영역은 이 자동 영역 안에 있어야 하며
최소 크기는 80×60이다. 처음에는 가용 영역을 사용하므로 크기를 줄인 뒤 남은 공간 안에서
옮길 수 있다. 더 넓은 영역이 필요하면 캔버스를 키우거나 제목·라벨 공간을 줄인다.
원형 차트는 이 영역 안에 원과 라벨을 배치하며 분할 원도 함께 이동한다. 막대·박스플롯은
영역을 데이터 좌표의 투영 범위로 쓰며, 축·눈금·그리드·데이터 라벨을 함께 다시 계산한다.
차트 제목과 범례는 각각 기존 캔버스 기준 배치를 유지한다.

DPR이나 PNG 배율은 저장한 논리 영역을 바꾸지 않는다. 캔버스 크기·글꼴·라벨이 바뀌어 지정한
영역이 들어가지 않으면 `prepare`가 거절한다. 자동 크기로 되돌리려면
`decorations.reset(&DecorationId::DataArea)` 또는 `data_area = None`을 사용한다.
핸들의 기하 제약을 벗어난 조정은 원자적으로 거절한다.
위 예제처럼 복사본에서 준비가 성공한 뒤 원본에 반영하면 실패 시
마지막 유효 화면과 설정을 유지할 수 있다.

영역 히트맵은 가장 뒤에 등록한다. 별도 데이터 피킹과 조합하는 호스트는 `DataArea`에 맞았더라도
데이터가 있으면 데이터를 먼저 선택해야 한다. 네이티브 예제의 `Edit plot area`를 누르면
데이터 위에서도 영역을 옮길 수 있고, 다시 누르면 데이터 선택으로 돌아간다. 포인터를 누를 때
선택한 핸들을 드래그가 끝날 때까지 유지하고, 매 프레임 이동량만 넘긴다.

## SSOT와 캐시

`decorations.data_area`가 영역을, `decorations.offsets`가 각 요소의 자동 배치 결과에 더할 논리 픽셀 이동량을 보관한다.
`decorations.reset(&id)`는 해당 요소를 자동 위치로 되돌린다. 세 모델의 `serde` 옵션으로
이 상태를 직렬화·복원할 수 있으며, 이전 문서에 `decorations`가 없으면 기본 위치와 선택 없음으로 읽는다.
원형 조각 ID는 원본 배열 인덱스이므로 조각을 재정렬할 때 호스트가 관련 선택·이동 상태도 대응시켜야 한다.

영역·위치·장식 선택 변경은 라벨 텍스처 캐시를 무효화한다. 재질·데이터 호버처럼 글자 위치를
바꾸지 않는 변경은 이전처럼 라벨 텍스처를 재사용한다. 동일 모델·크기·배율이면 프레임을 재사용한다.
이전 `Arc<Frame>`의 그림과 히트맵은 새 모델을 편집해도 바뀌지 않는다.

기본 내부 원형 라벨은 호버 시 기존 셰이더의 이동과 라스터 캐시 재사용을 유지한다.
원형 라벨을 직접 옮기거나 선택 테두리를 표시하는 동안에는 내부 라벨과 테두리의 위치를 CPU에서
함께 계산한다. 이 상태에서 호버 높이가 바뀌면 라벨 라스터도 갱신한다. 텍스트를 여러 번 리사이즈해서
누적하는 방식은 사용하지 않는다.

## 기존 API 및 웹 연결

`Selectable`, `Draggable`, `Resizable`, `HitMap`은 상태 타입을 받도록 확장했고 기본 타입은
기존 `Config`다. 기존 Cartesian 호출은 그대로 사용한다. 새 차트는 `DecorationLayout`을 넘긴다.
세 차트 모델에 필드가 추가됐으므로 Rust에서 모든 필드를 명시한 구조체 리터럴은
`decorations: Default::default()`를 추가해야 한다. 0.16.0/모델 0.10.0에서 `DecorationLayout`의
모든 필드를 명시하는 리터럴에는 `data_area: None`도 추가한다. `DecorationId`를 빠짐없이
분기하는 코드는 `DataArea`, `DataAreaEdge(Side)`를 처리해야 한다. `DataAreaEdge`는 리사이즈
내부의 변이 대상이며 별도 히트 영역으로 등록하지 않는다. 이전 JSON은 영역을 자동 배치로 읽는다.

이 문서는 renderer crate의 Rust API다. Cartesian용 `FiggyChart`와 별도로 현재 WASM
래퍼는 원형·범주형 막대·박스플롯을 위한 `FiggyBoundedChart`를 제공한다. 이 클래스는
히트맵·ID·선택·드래그·크기 조정·출력을 노출하며, 호스트는 포인터 이벤트를 연결하고
SSOT 변경 후 새 프레임을 그린다. 초기 표시는 `first_frame_ready()`를 기다린다.
[WASM API](WASM.md)를 참고한다. 이 소스 변경만으로 figgyplot.com 배포본이 갱신되는 것은 아니다.

작동하는 네이티브 연결 예제는 `radial_editor`, `categorical_editor`, `boxplot_editor`다.
제목·범례·라벨을 클릭해 선택하고 드래그하며, `Reset position`으로 되돌릴 수 있다.

## Independent bounded chart titles

`RadialChart`, `CategoricalChart` and `BoxPlotChart` accept an optional
`chart_title: BoundedTitleOptions { visible, text: RichText }`. When present it
is authoritative. The legacy plain `title` is a fallback only. Call
`normalize_title()` once when loading an old document into an editor: it copies
its title, family, color and former `font_size * 1.5` size into the independent
rich document. Later label size/color/family edits do not alter that document.
Native code can set `chart_title` directly; the old plain API remains available.

The existing `DecorationId::ChartTitle` and `decorations.offsets` remain the only
position authority. Do not add a second host-side title offset or HTML overlay.
Renderer layout, annotation cache keys, hit/selection bounds and PNG export use
the same rich text metrics, including per-segment styles and output scale.
Hidden titles are not registered in the hit map. Automatic plot space accounts
for title metrics; incompatible explicit data areas fail preparation atomically.

Title font sizes are 6–160 logical pixels, with at most 512 characters and eight
lines. The label font fields still control axis/category/data/legend text as
specified by each bounded family. They no longer control an explicit title.
CPU layout tests change label and title styles independently and inspect actual
annotation pixels and glyph bounds at 1× and 2× output scale.


Multi-line titles also support `text.align` (`TextAlign::Left`, `Center`, or
`Right`) inside the measured text block. `normalize_title()` preserves legacy
appearance by using `Left`; existing saved rich titles without `align` also
read as `Left`. Explicit Rust chart literals need `chart_title: None` (or use
`..Default::default()`). This restores a previously unmerged Git-only API; it
is not present in the published 0.17.0 archive.

## Layer and annotation foundation

Every chart family (`Config`, `RadialChart`, `CategoricalChart`, `BoxPlotChart`)
carries `active_layer: LayerRef` and `annotations: AnnotationLayer`. Both are
`serde(default)` and omitted from JSON at their defaults, so existing documents
parse and serialize unchanged. `LayerRef::Base` is the single layer every chart
has today; `Overlay(index)` and `Annotations` address the overlay layers and
the dedicated annotation layer that the follow-up features add. The active
layer is document state one level above element selection: it will decide
which layer's elements the hit map registers, while `selected`, `picked_points`
and `picked_data` keep naming the element inside that layer. Drawn pixels never
depend on `active_layer`.

`DecorationId` gains two variants for those layers. `Overlay { layer, inner }`
wraps an element identity with its overlay index (`overlay_<n>:<inner id>`) and
`Annotation(index)` names an annotation item (`annotation_<n>`). No hit map
registers them yet; hosts that match on `DecorationId` should treat them like
any other unknown decoration. Explicit Rust chart literals need the two new
fields (or `..Default::default()`). The bounded frame and label raster caches
strip both fields from their keys, so editing either reuses the cached frame;
only the layer's own pass will depend on them.

Run `cargo run --locked -p figgy-renderer --example bounded_titles` for a complete
native PNG example covering all three families, multiline alignment and a
subscript. See [the example source](examples/bounded_titles.rs). Hosts still
own editor widgets, Undo and file storage; no HTML overlay is required.
