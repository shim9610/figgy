# Radar charts

`RadarRenderer` draws radar (spider) charts: a few series compared over a
fixed set of categories, one spoke per category, each series a closed polygon
over one shared value axis. It is a bounded family like `RadialRenderer`,
`CategoricalRenderer` and `BoxPlotRenderer`: the whole chart is a small
`RadarChart` value, drawn by its own pipeline into an immutable frame, with no
GPU column pool, auto-fit job or stream replay.

## Input and first render

```rust,no_run
use renderer::radar::{RadarChart, RadarRenderer, RadarSeries};
use renderer::{Category, Color, RendererDevice};

fn draw(device: RendererDevice, format: wgpu::TextureFormat) -> Result<(), Box<dyn std::error::Error>> {
    let mut chart = RadarChart::default();
    chart.title = "Build quality".into();
    chart.categories = ["Speed", "Safety", "Size", "Docs", "Tests"]
        .iter()
        .map(|c| Category::new(c.to_lowercase(), *c))
        .collect();
    chart.series = vec![
        RadarSeries::new("v1", "Release 1", vec![6.0, 8.0, 4.0, 5.0, 7.0], Color::from_rgb8(60, 110, 200)),
        RadarSeries::new("v2", "Release 2", vec![8.0, 7.0, 6.0, 8.0, 9.0], Color::from_rgb8(220, 90, 70)),
    ];
    let mut renderer = RadarRenderer::new(device, format)?;
    let frame = renderer.prepare(&chart, (720, 520), 1.0)?;
    // In a render pass of a 720 x 520 attachment in `format`:
    // frame.draw(&mut pass);
    let _ = frame;
    Ok(())
}
```

- `categories[i]` owns spoke `i`; `series.values[i]` is the value on it. The
  spokes start at `style.start_angle_degrees` (-90, straight up) and follow
  `style.clockwise`. 3 to 64 categories and 1 to 16 series.
- The value axis is shared by every spoke. `value_range: None` fits the
  supplied values on a nice range that starts at zero (or below the smallest
  negative value) and ends on a ring; `Some([min, max])` fixes it. Values below
  the start sit at the centre, values above the end on the outer ring.
  `ring_spacing` chooses the ring interval; `None` picks a nice spacing for
  about five rings. Per-spoke scaling is the host's job: write normalized
  values and label the spokes accordingly.
- A non-finite value breaks the outline at that spoke, drops the series' fill
  and draws no marker there.
- `style.grid_shape` draws the rings as polygons through the spokes
  (`Polygon`, the spider web) or as circles. `grid: false` hides rings and
  spokes.
- Each series draws a fill (`RadarSeriesStyle::fill`, or the series colour at
  `style.fill_opacity`), an outline of `line_width_px` and a marker at every
  vertex (`Circle`, `Square` or `None`). Fills draw first for every series,
  then outlines, then markers, so a later series' fill never covers an earlier
  outline.
- `labels` writes the value at every vertex (`Values`) or at the vertices of
  the hovered or selected series only (`Emphasized`), formatted with
  `label_decimals` and `value_suffix`.

## Picking and selection

`RadarFrame::hit_test(position)` takes chart-local logical pixels and returns a
`RadarPick { target, part }`: a `Marker` hit names the vertex
(`RadarTarget { series_id, category_id: Some(..) }`), an `Outline` or `Fill`
hit names the series (`category_id: None`). Markers are tested first, then
outlines, then interiors, each in reverse draw order. Store the target in
`hovered` / `selected`: the series brightens by `style.emphasis_brightness`
and a picked vertex grows its marker. `RadarChart::value(&target)` reads the
value behind a vertex target.

## Decorations and annotations

The title, the value labels along the first spoke, the category labels at the
spoke ends (`DecorationId::SpokeLabel { category_id }`), the legend and the
data labels are measured decorations: `frame.decorations()` is the common
`HitMap` / `Draggable` contract, so they drag and reset like every other
bounded family's. The data area resizes the whole plot; the outer ring is the
largest circle that leaves room for the category labels inside it.

The annotation layer draws around the frame through `prepare_annotations`.
Anchors are ratios of the square around the outer ring; a data anchor has no
separable axis and is reported as a skip.

## Caching and export

`prepare` reuses its frame while every field the pixels depend on is unchanged
(`active_layer` and the annotation layer are not among them) and reuses the
label raster across hover and selection edits. `export_rgba_async` renders at
any scale and reads back straight-alpha RGBA8; `renderer::encode_png` encodes
it.

## Examples and tests

```bash
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo run --locked -p figgy-renderer --features egui_demo --example radar_editor
cargo test --locked -p figgy-renderer --test radar_render
```

## 한국어 사용법

레이더(거미줄) 차트는 고정된 범주 집합 위에서 몇 개의 시리즈를 비교한다. `categories[i]`가
스포크 `i`를 맡고, 각 시리즈의 `values[i]`가 그 스포크의 값이다. 범주는 3~64개, 시리즈는
1~16개다. 스포크는 `style.start_angle_degrees`(-90, 위쪽)에서 시작해 `style.clockwise`
방향으로 돈다.

값 축은 모든 스포크가 공유한다. `value_range: None`이면 0(또는 가장 작은 음수)에서 시작해
보기 좋은 고리 간격으로 데이터 최댓값을 덮는 고리까지 맞춘다. `Some([min, max])`는 범위를
고정하고, `ring_spacing`은 고리 간격을 정한다. 범위 아래 값은 중심에, 위 값은 바깥 고리에
놓인다. 스포크별 정규화는 호스트가 미리 계산해 값으로 넣는다. 렌더러는 통계를 계산하지
않는다. 유한하지 않은 값은 그 스포크에서 외곽선을 끊고 채움과 마커를 생략한다.

`style.grid_shape`는 고리를 스포크를 지나는 다각형(`Polygon`) 또는 원(`Circle`)으로
그린다. `grid: false`는 고리와 스포크를 숨긴다. 시리즈마다 채움(`RadarSeriesStyle::fill`
또는 시리즈 색을 `style.fill_opacity`로), `line_width_px`의 외곽선, 꼭짓점 마커(`Circle`,
`Square`, `None`)를 그린다. 모든 시리즈의 채움을 먼저, 그다음 외곽선, 마지막에 마커를
그리므로 나중 시리즈의 채움이 앞 시리즈의 외곽선을 가리지 않는다. `labels`는 모든 꼭짓점
(`Values`) 또는 호버·선택한 시리즈의 꼭짓점(`Emphasized`)에 값을 적으며 `label_decimals`와
`value_suffix`를 따른다.

`RadarFrame::hit_test`는 차트 논리 픽셀을 받아 `RadarPick { target, part }`를 돌려준다.
마커를 맞히면 꼭짓점(`RadarTarget { series_id, category_id: Some(..) }`), 외곽선이나 채움을
맞히면 시리즈(`category_id: None`)다. 마커, 외곽선, 내부 순서로 각각 그린 순서의 역순으로
판정한다. 대상을 `hovered` / `selected`에 넣으면 해당 시리즈가 `style.emphasis_brightness`만큼
밝아지고 선택한 꼭짓점의 마커가 커진다. `RadarChart::value(&target)`는 꼭짓점 뒤의 값을 읽는다.

`prepare`는 픽셀에 영향을 주는 모든 필드가 같을 때 프레임을 재사용하고, 호버·선택 편집에는
라벨 래스터를 재사용한다. `export_rgba_async`는 임의 배율로 그려 straight-alpha RGBA8을
읽어오며 `renderer::encode_png`로 PNG를 만든다.

## 제목·축·라벨·범례 선택과 이동

데이터 도형의 `hit_test`와 별도로 `frame.decorations()`가 렌더러에서 측정한 히트맵을 제공한다.
제목, 첫 스포크를 따라 놓인 값 라벨 묶음(`AxisLabel(Side::Left)`), 스포크 끝의 범주 라벨
(`DecorationId::SpokeLabel { category_id }`), 범례, 데이터 라벨이 대상이다. 기존
`Selectable`·`Draggable` 계약으로 `chart.decorations`를 갱신하고 `prepare`한다. 그리기 영역을
조정하면 플롯 전체가 옮겨지고 크기가 바뀌며, 바깥 고리는 범주 라벨이 들어갈 자리를 남긴 가장
큰 원이다. 좌표 단위, 요소별 이동 규칙, 캐시·저장 복원 및 웹 연결은
[공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)를 참고한다.

### 주석 층

`chart.annotations`의 텍스트 상자·화살표는 `prepare_annotations`로 프레임 주위에 별도 패스로
그린다. 앵커는 바깥 고리를 둘러싼 정사각형의 비율이며, 분리된 축이 없으므로 데이터 앵커는
건너뛴 것으로 보고한다. `BoundedAnnotationFrame`을 `draw_underlay` → `frame.draw` →
`draw_overlay` 순서로 그리고 히트 맵은 `annotations()`로 얻는다. 자세한 계약은
[공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer)를 참고한다.
