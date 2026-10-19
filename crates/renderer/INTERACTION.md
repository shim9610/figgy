# 새 차트의 선택·이동·크기 조정

원형·범주형 막대·박스플롯·레이더는 데이터 모델이 서로 다르지만, 영역·제목·축·라벨·범례는
기존 `HitMap → Selectable → Draggable / Resizable` 계약을 사용한다. 포인터 이벤트 전달과
드래그 중 프레임 요청은 호스트가 담당한다. 클릭 영역이나 라벨 위치를 호스트에서 다시 계산하지 않는다.

## 데이터 선택과 장식 요소 선택

- `frame.hit_test([x, y])`는 기존대로 조각·막대·박스·레이더 다각형의 **데이터**를 고른다.
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
| 막대·박스·레이더 값 라벨 | `DataLabel { category_id, series_id }` | 해당 데이터 라벨만 이동. 재정렬해도 같은 ID에 적용 |
| 레이더 스포크 라벨 | `SpokeLabel { category_id }` | 바깥 고리 끝의 범주 이름 하나만 이동. 재정렬해도 같은 ID에 적용 |

원형에는 축과 범례를 그리지 않으므로 이들의 히트 영역도 없다. 레이더는 첫 스포크를 따라
놓는 값 라벨 묶음을 `AxisLabel(Left)`로 등록하고 축 선·축 제목은 없다. 숨긴 제목·라벨·범례는
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

극좌표 차트(`Config.polar`)에서는 축이 바깥 고리와 광선이다. 고리(`Axis(Bottom)`)는 데이터 경계라
히트 영역이 없고, 광선(`Axis(Left)`)은 드래그하면 중심을 축으로 돌아 `radial_label_angle_degrees`가
바뀐다. 각도 눈금 라벨은 포인터가 있는 스포크를 따라 안팎으로, 반지름 눈금 라벨은 광선 프레임에서
움직인다. 포인터 위치가 필요한 규칙이므로 `Draggable::drag_from(cfg, at, dx, dy)`와
`Config::nudge_from(element, Some(at), dx, dy)`가 추가됐다. 기본 구현은 `drag_by`로 넘어가고 다른
요소는 영향이 없으며, 웹 래퍼의 `on_move`는 누른 지점에 이동량을 누적한 포인터 위치를 넘긴다.
상단·우측 축과 그 제목은 등록하지 않는다. 자세한 규칙은 `CONFIG.md`의 `PolarOptions` 절에 있다.
제목·축 제목·범례·그리기 영역은 그대로다.

이 문서는 renderer crate의 Rust API다. Cartesian용 `FiggyChart`와 별도로 현재 WASM
래퍼는 원형·범주형 막대·박스플롯·레이더를 위한 `FiggyBoundedChart`를 제공한다. 이 클래스는
히트맵·ID·선택·드래그·크기 조정·출력을 노출하며, 호스트는 포인터 이벤트를 연결하고
SSOT 변경 후 새 프레임을 그린다. 초기 표시는 `first_frame_ready()`를 기다린다.
[WASM API](WASM.md)를 참고한다. 이 소스 변경만으로 figgyplot.com 배포본이 갱신되는 것은 아니다.

작동하는 네이티브 연결 예제는 `radial_editor`, `categorical_editor`, `boxplot_editor`,
`radar_editor`다.
제목·범례·라벨을 클릭해 선택하고 드래그하며, `Reset position`으로 되돌릴 수 있다.

## Independent bounded chart titles

`RadialChart`, `CategoricalChart`, `BoxPlotChart` and `RadarChart` accept an
optional `chart_title: BoundedTitleOptions { visible, text: RichText }`. When present it
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

Every chart family (`Config`, `RadialChart`, `CategoricalChart`, `BoxPlotChart`,
`RadarChart`) carries `active_layer: LayerRef` and `annotations: AnnotationLayer`. Both are
`serde(default)` and omitted from JSON at their defaults, so existing documents
parse and serialize unchanged. `LayerRef::Base` is the single layer every chart
has today; `Overlay(index)` and `Annotations` address the overlay layers (#11)
and the dedicated annotation layer below. The active layer is document state
one level above element selection: it decides which layer's elements the
pointer route registers, while `selected`, `picked_points` and `picked_data`
keep naming the element inside that layer. Drawn pixels never depend on
`active_layer`. The overlay layers route through the Cartesian hit map (see
"Overlay layers") and the annotation layer puts its own hit map on top of the
chart's (see "Pointer route by the active layer" below).

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
data area (see [`OverlayLayer`](CONFIG.md#overlaylayer-configoverlays)). The
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
the final state (`layer_raster_generations()` exposes the per-panel counts;
the renderer test suite asserts both properties). An edit of the shared
geometry — the group right band, the chart area, a shared axis — lifts every
panel's margins and refreshes them all. Selection boxes are composited into the
topmost panel's decoration texture, above every layer. The registry classifies
every `overlays` field as a raster edit (`raster_config_equal`); an overlay's
range, scale, direction, band or series list is also a geometry edit (the
`view` revision), its colours and legend are not.

**Drawing order and pixels.** Panels composite in layer order, each as
grid → data (then its picked highlights) → decoration, so the picked-highlight
order is "A data → A highlight → B data → B highlight". Overlay panels draw
no grid, X axis, title or colourbar and, in the Milkyway style, no backdrop.
Drawn pixels never depend on `active_layer`, and inactive layers are not
dimmed; editing chrome belongs to the host. `ChartView::render_warnings` merges
every panel's tick warnings, overlay ones tagged `WarningAxis::OverlayY(index)`.

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

**Limits of this version.** Overlays own no X axis (no twiny); nesting is one
level (overlays have no layers of their own); bounded charts cannot be overlay
layers; a nonresident streaming series listed by an overlay is rejected by
every registry entry point, and the stream request paths reject a document
with overlays as a whole. Subplots are charts of their own; see the next
section.
Run `cargo run --locked -p figgy-renderer --example overlay_axes` for a native
twin-axis export; see [the example source](examples/overlay_axes.rs).

## Subplots

A `Config` with `subplots` is one document with several charts: the parent
and every subplot, each a complete chart with its own area and axes (see
[`Subplot`](CONFIG.md#subplot-configsubplots)). Axes are independent unless a
subplot links its `bottom_x` or `left_y` to the parent or an earlier subplot;
a link group then has one range, scale, direction and tick spacing.

**Views and caches.** The parent's panels are drawn at the `panel_rect` a host
passes to `create_chart_view` / `refresh_axis*`; every subplot panel is drawn
at its own area moved by the same offset, with its own grid, decoration and
transform resources and its own raster cache (`layer_raster_generations()`
lists the parent's panels and then each subplot's). An edit of one chart
re-rasterizes that chart's panels only, a linked range edit exactly the
charts of its link group, and the cached frame stays byte-identical to a fresh
render. Each selection box is composited into the decoration texture of the
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
another not.

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
Run `cargo run --locked -p figgy-renderer --example subplots` for a native
2×2 export; see [the example source](examples/subplots.rs).

## Annotation layer

Reference lines, spans, text boxes and arrows live in
`annotations: AnnotationLayer { visible, items }` of every chart family (item
kinds, anchors, styles and validation are in the
[configuration reference](CONFIG.md#layerref--annotationlayer)). One shared CPU
raster pass draws the layer into two panel-sized textures: the *underlay*
(spans) is composited below the data and the *overlay* (lines, text, arrows and
the annotation selection box) above every decoration, with the fullscreen
textured pipeline the grid and decoration rasters already use. Family shaders
and decoration rasters are untouched. The fixed order is:

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
`AnnotationMap::space()` on any map). Cartesian charts convert data anchors
through `bottom_x` / `left_y`; bar and box charts through the category index
axis and the value axis; radial charts place ratio anchors only and report
data anchors as warnings. Drags are converted back through the same space,
so a data-anchored item follows later zooms and a ratio-anchored one stays
put. `Anchor::converted(&space, kind)` switches kinds without moving.

**Revisions and caches.** `annotations.items` has its own raster revision.
On registered Cartesian charts `ChartRenderStamp::needs_annotation_raster_since`
says when to call `Renderer::refresh_annotations(view, chart, panel_rect,
selected)`, which rewrites the two annotation textures and the annotation hit
map and nothing else (`ChartView::layer_stamps()` counts each layer's writes;
the cached view plus that refresh paints the same bytes as a fresh renderer).
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
`ChartView::annotation_map()` after a refresh, `BoundedAnnotationFrame::
annotations()` after `prepare_annotations`. It has the `DecorationMap` shape:
`hit_test`, `element` (index and measured text size), `find(index)`,
`bounds`, `selection_box`, `drag_by`, `hit_resize_handle`, `resize_by`,
`warnings`, `space`. Coordinates are logical pixels of the frame: chart-local
for bounded charts, the chart area's frame for Cartesian charts. Every item
has the common blue selection box; the handles it carries are the item's own:
a span's two edges (`W`/`E` or `N`/`S`), a reference line's two value handles,
an arrow's two endpoints (the endpoint further left is `W`, the other `E`;
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
  chart has no other layer whose elements could be excluded. Bars, boxes and
  slices are not part of either map and keep answering through the frame's own
  `hit_test` on every layer, as GPU data picking keeps addressing the base
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
the route is an annotation refresh only, and the cached layers paint the bytes
of a fresh render of the final state. The index-based methods
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
[the WASM guide](WASM.md). Their pointer methods (`hit_test`, `on_press`,
`on_move` and `decoration_bounds` on `FiggyChart`; `hit_decoration`,
`select_decoration`, `resize_handle` and `drag` on `FiggyBoundedChart`) follow
the route above and name items as `"annotation_<n>"`.

Run `cargo run --locked -p figgy-renderer --example bounded_titles` for a complete
native PNG example covering all three families, multiline alignment and a
subscript. See [the example source](examples/bounded_titles.rs). Hosts still
own editor widgets, Undo and file storage; no HTML overlay is required.

## Area fill and step lines on Cartesian line series

A Cartesian line series can fill the area under its line or between its line
and another series (`DataLineStyleConfig.fill`) and connect its samples with
step pieces (`DataLineStyleConfig.interpolation`); see the
[configuration reference](CONFIG.md#area-fill-and-step-lines). Interaction
contracts:

- The fill is a property of the line, not a hit target. `pick_point` and
  `pick_data` keep resolving the line's samples; the picked-point ring and the
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
  leaves the view and selection revisions alone, reuses the column
  allocations, and the first frame after the edit is what a fresh renderer
  draws for the final state.
- The legend mark of a filled series carries a band in the fill colour under
  its rule; `series_symbol_segments` emits it and `RichSegment.rule_band`
  stores it, inside the fixed-width symbol field, so legend hit bounds do not
  change.

