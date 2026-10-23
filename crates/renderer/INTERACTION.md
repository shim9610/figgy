# 상호작용 가이드 / Interaction guide

원형·범주형 막대·박스플롯·레이더는 데이터 모델이 서로 다르지만, 그리기 영역·제목·축·라벨·범례는
Cartesian 차트와 같은 `HitMap → Selectable → Draggable / Resizable` 계약을 사용한다. 포인터 이벤트 전달과
드래그 중 프레임 요청은 호스트가 담당한다. 클릭 영역이나 라벨 위치를 호스트에서 다시 계산하지 않는다.

Radial, categorical bar, box plot and radar charts are the bounded charts: each
renderer draws one validated model and takes no column sources. Their plot
area, title, axes, labels and legend use the same
`HitMap → Selectable → Draggable / Resizable` contract as Cartesian charts. The
host forwards pointer events and requests frames while dragging; it never
recomputes hit regions or label positions.

- `frame.hit_test([x, y])` picks data: slices, bars, boxes, radar vertices and
  polygons. `frame.decorations().hit_test([x, y])` picks the title, axes,
  labels, legend and plot area, and returns `DataArea` wherever no other
  decoration is hit, including over data.
- Test the selected plot area's resize handles first (`hit_resize_handle`),
  then foreground decorations, then data, then `DataArea`. The plot area wins
  over data only while the host is in a plot-area edit mode.
- Coordinates and deltas are chart-local logical pixels. Convert the screen
  coordinates of a CSS-scaled canvas to the chart's logical size; never pass
  DPR-scaled framebuffer coordinates.
- Store a decoration selection in `chart.decorations.selected`, pass each
  pointer move's delta to `drag_by` / `resize_by`, then `prepare` a new frame
  before drawing or hit-testing again. A `HitId` is valid only for the frame
  that produced it; keep a `DecorationId` for longer.
- `decorations.data_area` holds an explicit plot rectangle in logical pixels.
  It must fit inside the automatic area and be at least 80×60, and `prepare`
  rejects it when the canvas, fonts or labels no longer leave room.
  `decorations.reset(&DecorationId::DataArea)` restores automatic layout.

## 데이터 선택과 장식 요소 선택

- `frame.hit_test([x, y])`는 원형 조각·막대·박스·레이더 꼭짓점과 다각형 같은 데이터를 고른다.
- `frame.decorations().hit_test([x, y])`는 제목·축·라벨·범례와 그리기 영역(`DataArea`)을 고른다.
- 영역을 선택한 상태라면 `map.hit_resize_handle(id, [x, y])`로 리사이즈 핸들을 먼저 검사한다.
  핸들은 영역 경계 밖으로도 걸쳐 있다.
- 그다음 제목·라벨·축·범례 같은 앞쪽 요소, 데이터, `DataArea` 순서로 호스트가 판정한다.
  `decorations().hit_test`는 데이터 위에서도 `DataArea`를 돌려주므로, 결과가 `DataArea`라면
  `frame.hit_test`로 데이터를 먼저 확인한다. 영역 편집 모드를 켰을 때만 영역을 데이터보다 우선한다.
- 좌표와 이동량은 차트 내부의 논리 픽셀이다. CSS로 확대된 캔버스라면 화면 좌표를
  차트의 논리 크기로 변환한다. DPR을 곱한 framebuffer 좌표를 넘기지 않는다.

장식 요소 선택은 `chart.decorations.selected`에, 데이터 선택과 호버는 막대·박스플롯·레이더의
`selected`·`hovered`와 원형의 `interaction.selected`·`interaction.hovered`에 저장한다.
두 선택은 별개 상태이므로 한 종류만 선택하려면 호스트가 다른 쪽 선택을 명시적으로 지운다.

| 요소 | `DecorationId` | 이동 규칙 |
|---|---|---|
| 그리기 영역 | `DataArea` | 영역 이동과 8개 핸들 크기 조정. 도형·축·라벨을 다시 배치 |
| 차트 제목 | `ChartTitle` | 화면 방향으로 자유 이동 |
| 축 선 | `Axis(Side)` | 축에 수직인 방향만 이동. 데이터·그리드는 고정, 눈금 글자는 축을 따라 이동 |
| 눈금·범주 라벨 | `AxisLabel(Side)` | 해당 축의 라벨 묶음을 이동 |
| 축 제목 | `AxisTitle(Side)` | 회전된 제목도 포인터의 화면 방향으로 이동 |
| 범례 | `Legend` | 기호와 모든 행을 함께 이동 |
| 원형 조각 라벨 | `RadialLabel(RadialTarget)` | 이름·수치 줄을 함께 이동. 외부 라벨 연결선 끝도 갱신 |
| 막대·박스·레이더 값 라벨 | `DataLabel { category_id, series_id }` | 해당 데이터 라벨만 이동. 재정렬해도 같은 ID에 적용 |
| 레이더 스포크 라벨 | `SpokeLabel { category_id }` | 바깥 고리 끝의 범주 이름 하나만 이동. 재정렬해도 같은 ID에 적용 |

원형 차트는 축과 범례를 그리지 않으므로 그 히트 영역도 없다. 레이더는 첫 스포크를 따라
놓는 값 라벨 묶음을 `AxisLabel(Left)`로 등록하고 축 선·축 제목은 없다. 숨긴 제목·라벨·범례는
히트맵에 등록되지 않는다. 축 라벨은 Cartesian 차트와 같이 묶음 선택이며, 범주 이름 하나마다
독립적으로 이동하는 계약은 아니다. 영역 조정은 데이터 값이나 축 범위를 바꾸지 않는다.
드래그로 데이터 값을 바꾸는 기능은 없다.

## Manual label placement

Use the displayed frame's decoration hit map to select and drag labels. Each
radial label, each bar/box/radar value label and each radar spoke label can move
independently; axis tick labels move together as a group. Drag deltas are
floating-point logical pixels, so small fractional adjustments are supported. Store selection in `decorations.selected`,
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
// 경계가 있는 차트의 요소는 측정된 경계를 이미 갖고 있으므로 measure는 읽지 않는다.
let result = element.as_draggable().unwrap()
    .drag_by(&mut chart.decorations, &renderer::CpuTextMeasure::default(), 8.0, -4.0);
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
원형 차트는 이 영역 안에 원과 라벨을 배치하며 분할 원도 함께 이동한다. 레이더는 영역 안에서
범주 라벨이 들어갈 자리를 남긴 가장 큰 원을 바깥 고리로 쓴다. 막대·박스플롯은
영역을 데이터 좌표의 투영 범위로 쓰며, 축·눈금·그리드·데이터 라벨을 함께 다시 계산한다.
차트 제목과 범례는 각각 기존 캔버스 기준 배치를 유지한다.

DPR이나 PNG 배율은 저장한 논리 영역을 바꾸지 않는다. 캔버스 크기·글꼴·라벨이 바뀌어 지정한
영역이 들어가지 않으면 `prepare`가 거절한다. 자동 크기로 되돌리려면
`decorations.reset(&DecorationId::DataArea)` 또는 `data_area = None`을 사용한다.
핸들의 기하 제약을 벗어난 조정은 원자적으로 거절한다.
위 예제처럼 복사본에서 준비가 성공한 뒤 원본에 반영하면 실패 시
마지막 유효 화면과 설정을 유지할 수 있다.

`DataArea`는 히트맵에 가장 먼저 등록되므로 겹치는 다른 장식 요소보다 우선순위가 낮다.
네이티브 예제의 `Edit plot area`를 누르면 데이터 위에서도 영역을 옮길 수 있고, 다시 누르면
데이터 선택으로 돌아간다. 포인터를 누를 때 선택한 핸들을 드래그가 끝날 때까지 유지하고,
매 프레임 이동량만 넘긴다.

## 단일 원본(SSOT)과 캐시

`decorations.data_area`가 영역을, `decorations.offsets`가 각 요소의 자동 배치 결과에 더할 논리 픽셀 이동량을 보관한다.
`decorations.reset(&id)`는 해당 요소를 자동 위치로 되돌린다. `serde` 기능을 켜면 원형·범주형 막대·박스플롯·레이더
모델 모두 이 상태를 직렬화·복원할 수 있으며, 이전 문서에 `decorations`가 없으면 기본 위치와 선택 없음으로 읽는다.
원형 조각 ID는 원본 배열 인덱스이므로 조각을 재정렬할 때 호스트가 관련 선택·이동 상태도 대응시켜야 한다.

영역·위치·장식 선택 변경은 라벨 텍스처 캐시를 무효화한다. 재질·데이터 호버처럼 글자 위치를
바꾸지 않는 변경은 라벨 텍스처를 재사용한다. 동일 모델·크기·배율이면 프레임을 재사용한다.
이전 `Arc<Frame>`의 그림과 히트맵은 새 모델을 편집해도 바뀌지 않는다.

기본 위치의 내부 원형 라벨은 호버할 때 셰이더가 함께 옮기므로 라스터 캐시를 그대로 재사용한다.
원형 라벨을 직접 옮기거나 선택 테두리를 표시하는 동안에는 내부 라벨과 테두리의 위치를 CPU에서
함께 계산한다. 이 상태에서 호버 높이가 바뀌면 라벨 라스터도 갱신한다.

## 트레잇 상태 타입, 마이그레이션, 웹 연결

`Selectable`, `Draggable`, `Resizable`, `HitMap`은 상태 타입을 매개변수로 받으며 기본값은 `Config`다.
Cartesian 차트는 `Config`를, 원형·범주형 막대·박스플롯·레이더는 `DecorationLayout`을 상태로 넘긴다.
원형·범주형 막대·박스플롯·레이더 모델의 모든 필드를 명시한 Rust 구조체 리터럴에는
`decorations: Default::default()`가, `DecorationLayout`의 모든 필드를 명시한 리터럴에는
`data_area: None`이 필요하다. `..Default::default()`로 끝내도 된다. `DecorationId`를 빠짐없이
분기하는 코드는 `DataArea`, `DataAreaEdge(Side)`, `SpokeLabel`, `Overlay`, `Annotation`도 처리해야 한다.
`DataAreaEdge`는 리사이즈가 내부에서 옮기는 변을 가리키며 히트 영역으로 등록하지 않는다.
`data_area`가 없는 JSON은 영역을 자동 배치로 읽는다.

Cartesian용 `FiggyChart`와 별도로 WASM 래퍼는 원형·범주형 막대·박스플롯·레이더를 위한
`FiggyBoundedChart`를 제공한다. 이 클래스는 히트맵·ID·선택·드래그·크기 조정·출력을 노출하며,
호스트는 포인터 이벤트를 연결하고 SSOT 변경 후 새 프레임을 그린다. 초기 표시는
`first_frame_ready()`를 기다린다. [WASM API](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#bounded-chart-host)를 참고한다.

네이티브 연결 예제는 `radial_editor`, `categorical_editor`, `boxplot_editor`, `radar_editor`이며
`egui_demo` 기능이 필요하다.
`cargo run --locked -p figgy-renderer --features egui_demo --example radial_editor`처럼 실행한다.
제목·범례·라벨을 클릭해 선택하고 드래그하며, `Reset position`으로 되돌릴 수 있다.

## 극좌표 차트의 축 드래그

극좌표 차트(`Config.polar`)에서는 바깥 고리가 각도 축, 반직선이 반지름 축이다. 고리(`Axis(Bottom)`)는
데이터 경계라 히트 영역이 없고, 반직선(`Axis(Left)`)은 드래그하면 중심을 축으로 돌아
`radial_label_angle_degrees`가 바뀐다. 각도 눈금 라벨은 포인터가 있는 스포크를 따라 안팎으로,
반지름 눈금 라벨은 반직선 프레임에서 움직인다. 이 규칙에는 포인터 위치가 필요하므로
`Draggable::drag_from(cfg, measure, at, dx, dy)`와 `Config::nudge_from(element, measure, Some(at), dx, dy)`에
이동량을 잰 기준 포인터 위치 `at`(차트 내부 논리 픽셀)을 함께 넘긴다. `drag_from`의 기본 구현은
`at`을 무시하고 `drag_by`를 호출하므로, 극좌표 축과 눈금 라벨이 아닌 요소는 `drag_by`와 똑같이
움직인다. 웹 래퍼의 `on_move`는 누른 지점에 이동량을 누적한 포인터 위치를 넘긴다. 상단·우측 축은
그리지 않으므로 축 선·눈금 라벨·제목 모두 히트 영역이 없다. 각도나 반지름 눈금 라벨을 선택하면
원 전체를 감싸는 상자 하나 대신 라벨마다 그려진 자리에 선택 상자를 그린다
(`SelectionBox::outline()`이 `SelectionOutline::Parts`이고, 반직선도 선과 눈금마다 그려진 자리에 상자를 그린다). 차트 제목, 하단·좌측 축 제목, 범례,
그리기 영역은 일반 차트와 같이 움직인다.
자세한 규칙은 `CONFIG.md`의 `PolarOptions` 절에 있다.

## Independent bounded chart titles

The four bounded charts, `RadialChart`, `CategoricalChart`, `BoxPlotChart` and
`RadarChart`, accept an optional
`chart_title: BoundedTitleOptions { visible, text: RichText }`. When present it
is authoritative. The legacy plain `title` is a fallback only. Call
`normalize_title()` once when loading an old document into an editor: it copies
its title, family, color and former `font_size * 1.5` size into the independent
rich document. Later label size/color/family edits do not alter that document.
Native code can set `chart_title` directly; the old plain API remains available.

The existing `DecorationId::ChartTitle` and `decorations.offsets` remain the only
position authority. Do not add a second host-side title offset or HTML overlay.
Renderer layout, label raster cache keys, hit/selection bounds and PNG export use
the same rich text metrics, including per-segment styles and output scale.
Hidden titles are not registered in the hit map. Automatic plot space accounts
for title metrics; incompatible explicit data areas fail preparation atomically.

Title font sizes are 6–160 logical pixels, with at most 512 characters and eight
lines. The label font fields control axis, category, data and legend text as
specified by each bounded family, not an explicit title.

Multi-line titles also support `text.align` (`TextAlign::Left`, `Center`, or
`Right`) inside the measured text block. `normalize_title()` preserves legacy
appearance by using `Left`; existing saved rich titles without `align` also
read as `Left`. Explicit Rust chart literals need `chart_title: None` (or use
`..Default::default()`).

Run `cargo run --locked -p figgy-renderer --example bounded_titles -- target/bounded-titles`
for a native PNG example covering all four families, multiline alignment and a
subscript; it writes `pie.png`, `bar.png`, `box.png` and `radar.png`. See
[the example source](https://github.com/shim9610/figgy/blob/master/crates/renderer/examples/bounded_titles.rs). Hosts own editor widgets,
undo and file storage.

## Layers and the active layer

Every chart family (`Config`, `RadialChart`, `CategoricalChart`, `BoxPlotChart`,
`RadarChart`) carries `active_layer: LayerRef` and `annotations: AnnotationLayer`. Both are
`serde(default)` and omitted from JSON at their defaults, so existing documents
parse and serialize unchanged. `LayerRef::Base` (the default) is the chart's
own layer; `Overlay(index)` addresses an overlay layer, `Subplot(index)` and
`SubplotOverlay(subplot, overlay)` a subplot and an overlay inside it, and
`Annotations` the annotation layer. Bounded charts have no overlays or
subplots and read those values as `Base`. The active layer is document state
one level above element selection: it decides which layer's elements the
pointer route registers, while `selected`, `picked_points` and `picked_data`
keep naming the element inside that layer. Drawn pixels never depend on
`active_layer`: inactive layers are not dimmed, and editing chrome belongs to
the host. Overlay layers and subplots route through the Cartesian hit map (see
"Overlay layers" and "Subplots") and the annotation layer puts its own hit map
on top of the chart's (see "Pointer route by the active layer" below).

`DecorationId` has two variants for those layers. `Overlay { layer, inner }`
wraps an element identity with its overlay index (`overlay_<n>:<inner id>`) and
`Annotation(index)` names an annotation item (`annotation_<n>`). The Cartesian
hit map registers the `Overlay` ids for the active overlay (see "Overlay
layers"); annotation items are hit-tested through their own `AnnotationMap`,
which the pointer route tests on top while `Annotations` is active (see
"Annotation layer"). Explicit Rust chart literals need the two fields (or
`..Default::default()`). The bounded frame and label raster caches strip both
fields from their keys, so editing either reuses the cached frame; only the
annotation pass depends on them.

## Overlay layers

A Cartesian `Config` with `overlays` is one document drawn as several internal
panels: the base layer and one panel per overlay, each a plain single-layer
`Config` produced by `Config::expand()` that shares the parent's chart area and
data area (see [`OverlayLayer`](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#overlaylayer-configoverlays)). The
renderer, the registry and the export path all expand through that one
function; hosts never build panels themselves.

**Views and caches.** `ChartView` holds one grid texture, one decoration
texture and one transform uniform per panel (`overlay_count()`), created by
`create_chart_view` and kept in step by every `refresh_axis*` call: a layer
added since the last refresh gets its textures there, a removed one drops
them. Each panel caches the internal config, selection boxes, font generation
and panel rect of its last raster, so a refresh re-rasterizes only the panels
whose internal config changed. Editing one layer's axis, legend or range
leaves the other layers' textures untouched, and the frame drawn from the
cached view plus the refreshed layer is byte-identical to a fresh render of
the final state (`layer_raster_generations()` exposes the per-panel counts).
An edit of the shared geometry (the chart area, the X range, or an axis band
that changes `Config::margins`, such as the widest right Y axis band) changes
every panel's internal config and re-rasterizes every panel. Selection boxes are composited into the topmost panel's
decoration texture, above every layer. The registry classifies every
`overlays` field as a raster edit (it moves the `raster` revision that
`ChartRenderStamp::needs_raster_since` reports); an overlay's range, scale,
direction, band or series list is also a geometry edit (the `view` revision),
its colours and legend are not.

**Drawing order and pixels.** Panels composite in layer order, each as
grid → data (then its picked highlights) → decoration, so the picked-highlight
order is "A data → A highlight → B data → B highlight". Overlay panels draw
no grid, X axis, title or colourbar and, in the Milkyway style, no backdrop.
`ChartView::render_warnings` merges every panel's tick warnings, overlay ones
tagged `WarningAxis::OverlayY(index)`.

**Interaction target.** `Renderer::set_active_layer(chart, LayerRef)` and
`active_layer(chart)` choose the layer the hit map addresses; the model has the
same `Config::set_active_layer` / `active_layer()` for low-level hosts and the
WASM wrapper exposes `set_active_layer(json)` / `active_layer()`. Build the hit
map with `HitMap::for_active_layer(&config)`: `Base` is exactly
`standard_chart()`; an active overlay registers the shared elements first (data
area, both X axes with their label bands and titles, the merged legend, chart
title, colourbar) and its own elements last — `overlay_<n>:axis_right`,
`overlay_<n>:tick_labels_right`, `overlay_<n>:axis_title_right` and
`overlay_<n>:legend` — so the reverse-order test puts the active layer on
top. Inactive layers' elements are not registered, so clicks pass through: the
base Y axes do not answer while an overlay or the annotation layer is active.
The set depends only on the active layer and the overlay count; rebuild the map
when either changes. Overlay elements measure against the overlay's panel and
write a drag back into `overlays[n].y_axis` (line offset, label and title
offsets) or `overlays[n].legend` only. The data area is group geometry: its
drag and its eight handles move the shared rectangle through the parent's
margins and pull every overlay band along.

Changing the layer clears the element selection — a `HitId` belongs to the
previous layer's map — and advances the raster revision when there was one, so
the selection chrome is redrawn; the data revision is untouched. An `Overlay`
index past the overlay list is stored as `Base` by the setter and by every
config replacement, so deleting the active overlay recovers to `Base`. GPU
data picking (`pick_chart`, `pick_chart_data`) addresses the active layer: it
uses that layer's panel (its Y range, the group data area) and only the series
it scales; the base layer answers while the annotation layer is active.

**Linked axes.** X is shared by construction, so one `set_chart_config` or
`set_chart_view_state` call moves every layer; `ChartViewState::overlay_y`
carries one Y range per overlay and must have exactly as many entries as the
document has overlays. Every panel's transform is written from that one config
snapshot in the same `prepare`, so a frame in which the base moved and an
overlay did not cannot be produced. Auto-fit for an overlay axis uses the same
cached column statistics: `Chart::auto_fit_overlay_y_extent` /
`auto_fit_overlay_y_union` / `set_overlay_y_range` natively,
`auto_fit_overlay_y(layer, column, padding)` in the WASM wrapper.
The WASM `auto_fit_all(padding)` (also on `<figgy-chart>`) fits only the axes
of the active layer, each from the series drawn against it
(`Config::layer_of_series`). With `Base` or `Annotations` active (the
annotation layer draws on the base axes) it fits the parent's `bottom_x` and
`left_y`: an overlay shares the parent's X axis, so the overlays' series count
towards `bottom_x`, but each overlay has its own Y axis, so they do not count
towards `left_y`. With `Overlay(n)` active it fits only that overlay's Y axis,
from its series. Series of a subplot count only where an axis is linked (see
"Subplots"). To fit every layer, a host sets each layer active in turn
(`set_active_layer`, then `auto_fit_all`) or calls `auto_fit_overlay_y` /
`auto_fit_subplot` with explicit columns.

**Export.** `export_document_rgba_async` / `export_document_png_bytes_async`
(and their `_with_clear` and blocking native variants) render the whole group
into one image; the existing `export_panel_*` entry points stay and render the
base panel only. For a document without overlays both produce byte-identical
images. The WASM `export_png` exports the document.

**Annotations on a layered document.** The document has one annotation layer.
It is rasterized once, on the base view, with anchors in the parent's own axes
(`bottom_x` / `left_y`) and the group data area. Its underlay is composited
below the base panel's data and its overlay above the topmost panel's
decoration, so text boxes and arrows sit above every layer. Annotation edits
never invalidate a panel's axis raster cache. Overlay panels carry no
annotation textures.

**Limits.** Polar charts take no overlay layers; overlays own no X axis (no
twiny); nesting is one level (overlays have no layers of their own); bounded
charts cannot be overlay layers; a nonresident streaming series listed by an
overlay is rejected by every registry entry point, and the stream request
paths reject a document with overlays as a whole. Subplots are charts of
their own; see the next section.

Run `cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes`
for a native twin-axis export (`gallery-twin-axes.png`); see
[the example source](https://github.com/shim9610/figgy/blob/master/crates/renderer/examples/overlay_axes.rs).

## Subplots

A `Config` with `subplots` is one document with several charts: the parent
and every subplot, each a complete chart with its own area and axes (see
[`Subplot`](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#subplot-configsubplots)). Axes are independent unless a
subplot links its `bottom_x` or `left_y` to the parent or an earlier subplot;
a link group then has one range, scale, direction and tick spacing.

**Views and caches.** The parent's panels are drawn at the `panel_rect` a host
passes to `create_chart_view` / `refresh_axis*`; every subplot panel is drawn
at its own area moved by the same offset, with its own grid, decoration and
transform resources and its own raster cache (`layer_raster_generations()`
lists the parent's panels and then each subplot's). An edit of one chart
re-rasterizes that chart's panels only, and a linked range edit exactly the
charts of its link group. Each selection box is composited into the decoration texture of the
topmost panel whose rectangle holds the box's centre. The registry classifies
every `subplots` field as a raster edit; a subplot's area, data area, linked
or own ranges and series list are also geometry (the `view` revision).

**Interaction target.** `LayerRef::Subplot(i)` addresses subplot `i` and
`LayerRef::SubplotOverlay(i, j)` overlay `j` inside it. While one is active,
`HitMap::for_active_layer` registers exactly the set that subplot's own chart
registers for `Base` or `Overlay(j)`, each element wrapped as a
`SubplotElement` with a `subplot_<i>:` id prefix (`subplot_0:data_area`,
`subplot_0:axis_left`, `subplot_0:overlay_0:axis_right`, …), and none of the
parent's. Every element measures against the subplot's chart
(`Config::subplot_config`) and applies a drag or resize through its own
single-chart policy to that chart, stored back in `subplots[i].config`: the
subplot's data area moves and resizes through its own margins, its axes,
labels, titles and legend move as on any chart. The set depends on the active
layer, the overlay count and the active subplot's overlay count; rebuild the
map when any of them changes. A subplot or overlay index past its list
recovers to `Base`, a missing subplot overlay to its subplot. GPU picking of an
active subplot layer uses that subplot's panel in its own area and only the
series it draws.

**Linked axes.** `ChartViewState::subplots` carries one view state per
subplot (a linked axis reads as its root's range) and must match the
document's subplot and subplot-overlay counts. One `set_chart_view_state`,
`set_chart_config` or `set_chart_state` call carries an edit of any linked
axis to the root of its group and moves every member in the same call
(`Config::sync_subplot_links`), so no frame can show one linked chart moved and
another not. Auto-fit writes the root of a linked axis, so a fit moves the
whole link group: `Chart::auto_fit_subplot_{x,y}_{extent,union}` natively,
`auto_fit_subplot(index, axis, columns, padding)` in the WASM wrapper, fitted
to the columns the host names. The WASM `auto_fit_all(padding)` chooses the
series itself. With `Subplot(i)` active it fits subplot `i`'s X and Y axes;
with `SubplotOverlay(i, n)` active, only that overlay's Y axis. A linked axis
is one axis shared by its link group, so it fits the series of every chart in
the group, the parent included when the group's root is the parent, and the
range moves every member, whichever member is active. X and Y links are
followed separately: with subplot 1's `link_y` on subplot 0 and no `link_x`,
fitting either subplot gives both the Y range of their combined series, and
each keeps an X range fitted to its own series. A chart's overlays share its X
axis and count towards it, but not towards its `left_y`. An axis that is not
linked fits its own chart's series only. A base fit also moves the subplots
linked to the parent's axes. Natively, `Chart::auto_fit_layer_union(pool,
layer, series, padding)` applies the same rule (`Config::series_fit_roles`)
from column statistics, and `Chart::auto_fit_layer_extent` from exact
extents.

**Display, export and annotations.** `display_config_for_surface` fits the
whole document (`Config::document_area`, the union of every chart area) to the
surface and returns the rect where the parent's area lands;
`GpuPickRequest::display_panel_px` is the displayed document rect
(`fit_display_panel` of the document area). The document export renders the
union; `export_panel_*` keeps the parent's area. The document's one annotation
layer stays in the parent's area: it is drawn there only, anchored in the
parent's axes, and its items answer the pointer route
(`ChartView::hit_test_active_layer`) only while `Annotations` is active; with
a subplot active the route reaches the subplot's own elements.
`ChartView::render_warnings` tags a subplot's warnings
`WarningAxis::Subplot { index, axis }` (`subplots[i].config.<axis>`).

**Limits.** Subplots nest one level (a subplot's chart has overlays but no
subplots), are Cartesian only, keep no annotation layer of their own, and a
nonresident streaming series drawn by a subplot is rejected like one on an
overlay; the stream request paths reject a document with subplots as a whole.
Run `cargo run --locked -p figgy-renderer --example subplots -- target/subplots`
for two native 2×2 exports: `gallery-subplots.png`, the parent and three
subplots linked to its X axis, and `mixed-axes.png` with one linked axis, a
different X column and a twin Y axis inside a subplot. See
[the example source](https://github.com/shim9610/figgy/blob/master/crates/renderer/examples/subplots.rs).

## Annotation layer

Reference lines, spans, text boxes and arrows live in
`annotations: AnnotationLayer { visible, items }` of every chart family (item
kinds, anchors, styles and validation are in the
[configuration reference](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#layerref--annotationlayer)).
One shared CPU raster pass draws the layer into two panel-sized textures: the
*underlay* (spans) is composited below the data and the *overlay* (lines,
text, arrows and the annotation selection box) above every decoration, with
the fullscreen textured pipeline the grid and decoration rasters use. The
fixed order is:

- Cartesian panels: grid → annotation underlay → data → picked highlight →
  decoration → annotation overlay.
- Bounded charts: annotation underlay → the frame's own draw → annotation
  overlay. The frame paints an opaque background by default, so an underlay
  only shows through a frame prepared with
  `FrameBackground::HostClear` (`set_background`), which leaves the background
  to the host's clear colour; the host then clears with `chart.background`.
  `export_rgba` switches to that mode by itself whenever an underlay exists and
  keeps one cached frame per mode, so an export never evicts the interactive
  frame.

A text box may overlap the legend; the user resolves that by moving the box.
Annotations do not take part in the data-area margins and the label placer
never nudges them.

**Anchor space.** The only thing the layer needs from a chart family is the
anchor ↔ pixel conversion, the `AnnotationSpace` trait implemented by the
`PixelSpace` each frame derives from its layout (`pixel_space_for_config` for
Cartesian configs, `Frame::annotation_space()` for bounded frames,
`AnnotationMap::space()` on any map). Rectangular Cartesian charts convert
data anchors through `bottom_x` / `left_y`; bar and box charts through the
category index axis and the value axis. Polar charts (`Config.polar`), radial
charts and radar charts have no separable value axes: they place ratio
anchors only and report data anchors as warnings. Drags are converted back
through the same space, so a data-anchored item follows later zooms and a
ratio-anchored one stays put. `Anchor::converted(&space, kind)` switches kinds without moving.

**Revisions and caches.** `annotations.items` has its own raster revision.
On registered Cartesian charts `ChartRenderStamp::needs_annotation_raster_since`
says when to call `Renderer::refresh_annotations(view, chart, panel_rect,
selected)`, which rewrites the two annotation textures and the annotation hit
map and nothing else (`ChartView::layer_stamps()` counts each layer's writes).
`refresh_axis*` redoes the annotation rasters too, because the anchor space
follows the axes and the data area, so a host that refreshes the axis raster
needs no second call for the same frame. Toggling `visible` bumps no raster
revision at all: the draw skips the two composites. On bounded charts
`prepare_annotations(chart, size, scale)` returns an
`Arc<BoundedAnnotationFrame>` cached beside the frame: an item edit or a
selection change returns the same `Arc<Frame>` with the same label raster and
a new annotation frame; a `visible` toggle returns an annotation frame that
`shares_rasters_with` the previous one.

**Hit map, selection and handles.** The layer owns a `HitMap<AnnotationLayer>`
(`AnnotationLayer` implements `DragConfig`) exposed as an `AnnotationMap`:
`ChartView::annotation_map()` after a refresh,
`BoundedAnnotationFrame::annotations()` after `prepare_annotations`. It has the
`DecorationMap` shape: `hit_test`, `element` (index and measured text size),
`find(index)`, `bounds`, `selection_box`, `drag_by`, `hit_resize_handle`,
`resize_by`, `warnings`, `space`. Coordinates are logical pixels of the frame:
chart-local for bounded charts; for Cartesian charts, the same pixel space as
the `HitMap`, where `chart_area` sits at its own `x` / `y`. Every item has the
common blue selection box; the handles it carries are the item's own: a span's
two edges (`W`/`E` or `N`/`S`), a reference line's two value handles (`W`/`E`
on an `X` line, `N`/`S` on a `Y` line; both move the value), an arrow's two
endpoints (the endpoint further left is `W`, the other `E`;
`N`/`S` for a vertical arrow); a text box has none. `resize_by` with that
compass name moves the handle, `drag_by` moves the whole item. Moves that
would leave the axis range (lines, span edges), the plot (a whole span) or the
panel (text, arrow endpoints) are rejected atomically. Later items win where
items overlap. Hidden items and items the raster skipped are not registered;
the skipped ones are `RenderWarning::AnnotationSkipped` on
`ChartView::render_warnings()` / `BoundedAnnotationFrame::render_warnings()`.

Selection follows each family's convention. Bounded charts store it in the
shared `decorations.selected = Some(DecorationId::Annotation(index))`; the
frame caches ignore that value, so selecting an annotation never rebuilds the
chart frame. Cartesian hosts pass the selected index to `refresh_annotations`;
the view remembers it for later axis refreshes. The box is drawn on the
overlay, above everything.

**Pointer route by the active layer.** The annotation layer is
`LayerRef::Annotations` and uses the same `active_layer` API as the overlay
layers. While it is active and shown, the drawn items sit on top of the
chart's own hit map, so the reverse-order rule puts them above everything;
while any other layer is active they are not part of the route and clicks
pass through them. Two maps make one route because the item bounds are
measured by the renderer into the annotation layer's own
`HitMap<AnnotationLayer>`:

- Cartesian: `ChartView::hit_test_active_layer(&config, &hit_map, point)`
  tests the view's annotation map first and then `hit_map`, the
  `HitMap::for_active_layer(&config)` map. For `Annotations` that map holds the
  shared elements (data area, both X axes with their labels and titles, the
  merged legend, the chart title and the colourbar) and no Y axis of the base
  or any overlay, in a single-layer chart and in a layered document alike.
  `config` is the config the view was refreshed with (the display config of a
  scaled view).
- Bounded: `BoundedAnnotationFrame::hit_test_active_layer(chart.active_layer(),
  frame.decorations(), point)` tests the items above every decoration of the
  frame; the decorations all stay registered below them, because a bounded
  chart has no other layer whose elements could be excluded. Slices, bars,
  boxes and radar vertices and polygons are not part of either map and keep
  answering through the frame's own `hit_test` on every layer, as GPU data picking keeps addressing the base
  layer on Cartesian charts.

Both return a `LayerHit`: `Annotation { index, id }` (select
`DecorationId::Annotation(index)` and drag through the annotation map with
`id`) or `Element(id)` of the chart's own map. `AnnotationMap::route(active,
layer, point, below)` is the shared rule for hosts that keep their own maps;
`AnnotationMap::receives_pointer(active, layer)` says whether the items take
part, which a hidden layer (`visible = false`) never does.

Changing the layer clears the element selection, as on overlay layers: the
registry drops its `HitId`, the WASM wrapper drops the annotation selection
when the layer changes away from or into `Annotations`, and the bounded
`set_active_layer` (`RadialChart`, `CategoricalChart`, `BoxPlotChart`, `RadarChart`,
with an `Overlay` stored as `Base`) clears `decorations.selected`. The data selection
(`picked_*`, the bounded charts' selected bar, box, slice or radar target) is kept. A layer
switch changes no raster revision of its own: the data, grid and decoration
textures and the bounded frame and label raster are reused, and only a cleared
selection redraws its selection chrome. Selecting or dragging an item through
the route is an annotation refresh only. The index-based methods
(`AnnotationMap::find`, `drag_by`, `resize_by` and the WASM `*_annotation`
methods) keep working on every layer for hosts with their own annotation
tool.

```rust,no_run
use renderer::{Anchor, Annotation, AnnotationAxis, AnnotationKind, DecorationId};
# fn edit(renderer: &mut renderer::CategoricalRenderer, chart: &mut renderer::CategoricalChart)
# -> Result<(), renderer::CategoricalError> {
chart.annotations.items.push(Annotation::new(AnnotationKind::Span {
    axis: AnnotationAxis::X, from: -0.5, to: 0.5, fill: renderer::Color::new(1., 0., 0., 0.3),
}));
chart.annotations.items.push(Annotation::new(AnnotationKind::Text {
    anchor: Anchor::Ratio { x: 0.6, y: 0.1 },
    text: renderer::text::RichText::plain("note", renderer::Color::BLACK, 12., ""),
    box_style: Some(Default::default()),
}));
// The span is an underlay: prepare the frame with a host clear and clear the
// pass with `chart.background`.
renderer.set_background(renderer::FrameBackground::HostClear);
let frame = renderer.prepare(chart, (800, 520), 2.0)?;
let layer = renderer.prepare_annotations(chart, (800, 520), 2.0)?;
if let Some(hit) = layer.annotations().hit_test([400.0, 60.0]) {
    let index = layer.annotations().element(hit).unwrap().index;
    chart.decorations.selected = Some(DecorationId::Annotation(index));
    // Frame delta in logical pixels; the anchor is written back through the frame's space.
    let _ = layer.annotations().drag_by(&mut chart.annotations, hit, 8.0, -4.0);
}
let layer = renderer.prepare_annotations(chart, (800, 520), 2.0)?;
// In the pass: layer.draw_underlay(pass); frame.draw(pass); layer.draw_overlay(pass).
# let _ = frame; let _ = layer;
# Ok(()) }
```

The Cartesian call order is the usual one plus the annotation branch:
`refresh_axis*` when the raster stamp changed (it carries the annotations
along), else `refresh_annotations` when the annotation stamp or the selection
changed, then `prepare` / `paint`. `export_panel_*` rasterizes the layer at the
export scale from the same scaled config as the axes, so 1× and 2× exports use
the metrics of their hit maps.

The WASM wrappers expose the layer on both classes (`FiggyChart`,
`FiggyBoundedChart`): `annotations()`, `hit_annotation(x, y)`,
`select_annotation(index | null)`, `selected_annotation()`,
`annotation_handle(index, x, y)`, `drag_annotation(index, dx, dy, handle)`,
`add_annotation(json)`, `set_annotation(index, json)`,
`remove_annotation(index)`, `set_annotations_visible(bool)`,
`pointer_to_anchor(x, y, "Data" | "Ratio")` and `annotation_warnings()`; see
[the WASM guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#bounded-chart-host). Their pointer methods (`hit_test`, `on_press`,
`on_move` and `decoration_bounds` on `FiggyChart`; `hit_decoration`,
`select_decoration`, `resize_handle` and `drag` on `FiggyBoundedChart`) follow
the route above and name items as `"annotation_<n>"`.

## Area fill and step lines on Cartesian line series

A Cartesian line series can fill the area under its line or between its line
and another series (`DataLineStyleConfig.fill`) and connect its samples with
step pieces (`DataLineStyleConfig.interpolation`); see the
[configuration reference](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#area-fill-and-step-lines).
Interaction contracts:

- The fill is a property of the line, not a hit target. `pick_chart` and
  `pick_chart_data` (`pick_point` and `pick_data` in the WASM wrapper) keep
  resolving the line's samples; the picked-point ring and the
  typed selection overlay are drawn after every fill and every body, so a fill
  never covers a highlight. Selecting the series selects its fill.
- A pick on a stepped line walks the same axis-aligned pieces the line draws.
  The nearest piece wins and the result snaps to the nearer source index by
  position along the stepped path, so a hit on a riser still names one of the
  two samples it joins; the straight chord between two samples is not
  pickable in a step mode. CPU and GPU pickers agree on this, including the
  data-space midpoint of `StepMid`.
- Editing `fill`, `fill_color` or `interpolation` is a series declaration
  change: the registry moves the data revision (like any `render_type` edit),
  leaves the view and selection revisions alone and reuses the column
  allocations.
- The legend mark of a filled series carries a band in the fill colour under
  its rule; `series_symbol_segments` emits it and `RichSegment.rule_band`
  stores it, inside the fixed-width symbol field, so legend hit bounds do not
  change.

