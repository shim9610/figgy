# Radar charts

`RadarRenderer` draws radar (spider) charts: a few series compared over a
fixed set of categories, one spoke per category, each series a closed polygon
over one shared value axis. Like `RadialRenderer`, it draws a small chart model with its own renderer: the
whole chart is one `RadarChart` value, with no `ColumnSource`, and
`RadarRenderer::prepare` draws it into an immutable frame. It does not use the
Cartesian `Renderer`, its column pool, auto-fit or streamed sources.

## Input and first render

```rust,no_run
use renderer::radar::{RadarChart, RadarRenderer, RadarSeries};
use renderer::{Category, Color, RendererDevice, encode_png};

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

    // On screen: draw the frame in the host's render pass, on a single-sample
    // attachment of `frame.size()` (720 x 520 at scale 1.0) in `format`.
    let frame = renderer.prepare(&chart, (720, 520), 1.0)?;
    // frame.draw(&mut pass);
    let _ = frame;

    // Headless: render, read back straight-alpha RGBA8 and encode a PNG.
    let image = renderer.export_rgba(&chart, (720, 520), 1.0)?;
    std::fs::write("radar.png", encode_png(&image)?)?;

    // After every command buffer that drew a frame is submitted or discarded.
    renderer.end_frame();
    Ok(())
}
```

`RadarRenderer::new` accepts an RGBA8 or BGRA8 target format, linear or sRGB.
`prepare(&chart, size, scale)` takes a logical size of at least 240 x 240 and a
scale from 0.5 to 4; the frame is `size` times `scale` physical pixels and must
fit the device's texture limit. A format, size or scale outside these limits,
or a chart that fails `RadarChart::validate`, returns `RadarError::Invalid`.

- `categories[i]` owns spoke `i`; `series.values[i]` is the value on it, so
  every series has exactly one value per category. A chart has 3 to 64
  categories and 1 to 16 series. The spokes start at
  `style.start_angle_degrees` (-90, straight up) and run clockwise while
  `style.clockwise` is `true` (the default).
- The value axis is shared by every spoke. `value_range: None` fits the
  supplied values on a nice range that starts at zero (or below the smallest
  negative value) and ends on a ring; `Some([min, max])` fixes it. Values below
  the start sit at the centre, values above the end on the outer ring.
  `ring_spacing` chooses the ring interval; `None` picks a nice spacing for
  about five rings. Rings are the multiples of the spacing inside the range,
  walked by the shared `model::tick::LinearGrid`, so a ring that rounding puts
  just past the end (0.3 with spacing 0.1) is kept; ring labels use the
  automatic `PlainNumberFormat` form. Per-spoke scaling is the host's job: write normalized
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
- `labels` is `RadarLabels::None` by default, with no data labels. `Values`
  writes the value at every vertex and `Emphasized` only at the vertices of the
  hovered or selected series, formatted with `label_decimals` and
  `value_suffix`.

## Picking and selection

`RadarFrame::hit_test(position)` takes chart-local logical pixels and returns a
`RadarPick { target, part }`: a `Marker` hit names the vertex
(`RadarTarget { series_id, category_id: Some(..) }`), an `Outline` or `Fill`
hit names the series (`category_id: None`). Markers are tested first, then
outlines, then interiors, each in reverse draw order. Store the target in
`hovered` / `selected`: the series brightens by `style.emphasis_brightness`
and a picked vertex grows its marker. `RadarChart::value(&target)` returns the
value at a vertex target (`None` for a series target).

In the browser, `FiggyBoundedChart` loads the same model as a `"radar"`
document. Its `step_selection(backwards)` walks the series polygons in order,
or, while a vertex is selected, that series' finite vertices in category order
(see the [bounded chart host](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#bounded-chart-host)).

## Decorations and annotations

The title, the value labels along the first spoke
(`DecorationId::AxisLabel(Side::Left)`, moved as one group), the category
labels at the spoke ends (`DecorationId::SpokeLabel { category_id }`), the
legend and the data labels are measured decorations. `frame.decorations()`
follows the `HitMap` / `Draggable` contract of the pie, bar and box charts:
`hit_test(position)` finds an element, `drag_by(&mut chart.decorations, hit,
dx, dy)` moves it, `chart.decorations.reset(&id)` restores its automatic
place, and the next `prepare` draws the result. The data area
(`DecorationId::DataArea`) moves and resizes the whole plot through
`hit_resize_handle` and `resize_by`; the outer ring is the largest circle that
leaves room for the category labels inside it. Coordinates, selection and the
rules per element are in the
[interaction guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md).

`renderer.prepare_annotations(&chart, size, scale)` returns the annotation
layer (`BoundedAnnotationFrame`) for the frame `prepare` returns with the same
arguments. Draw `draw_underlay`, then `frame.draw`, then `draw_overlay` into
the same attachment; `annotations()` is the layer's hit map. When
`draws_underlay()` is `true`, call
`renderer.set_background(FrameBackground::HostClear)` before `prepare` and
clear the pass with `chart.background`, or the frame paints its background
over the underlay. Text boxes and arrows use ratio anchors over the square
around the outer ring. A data anchor, a reference line and a span have no
value axis to land on; `render_warnings()` reports them as
`RenderWarning::AnnotationSkipped` with `AnnotationSkip::NoValueAxis`. The full contract is in the
[annotation layer guide](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer).

## Caching and export

`prepare` reuses its frame while every field the pixels depend on is unchanged
(`active_layer` and the annotation layer are not among them), so model edits
need no manual cache invalidation. It reuses the label raster across hover,
selection, background and series-style edits; with `labels: Emphasized`, a
hover or selection change redraws the labels.

After submitting or discarding every command buffer that drew a frame, old
snapshots included, call `renderer.end_frame()` so the GPU memory of replaced
frames is released when the queue completes; native hosts also poll the
device. The renderer has its own 256 MiB budget, separate from the Cartesian
renderer's (`set_memory_budget`, `gpu_memory_usage`). Memory still awaiting
`end_frame` counts against it, and `prepare` returns `RadarError::Budget`
instead of exceeding it. `clear_cache` drops the renderer's references to its
cached frames; frames the host holds stay valid.

`export_rgba_async` (and its blocking native form `export_rgba`) renders the
chart with its annotation layer and current hover and selection through
`prepare`, so the same size and scale limits apply, and reads back
straight-alpha RGBA8; `renderer::encode_png` encodes it.

## Examples and tests

```bash
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo run --locked -p figgy-renderer --features egui_demo --example radar_editor
cargo test --locked -p figgy-renderer --test radar_render
```

## 한국어 사용법

레이더(거미줄) 차트는 시리즈 몇 개를 정해진 범주별로 비교한다. `categories[i]`가 스포크 `i`를
맡고 각 시리즈의 `values[i]`가 그 스포크의 값이므로, 시리즈마다 범주 수만큼 값이 있어야 한다.
범주는 3~64개, 시리즈는 1~16개다. 스포크는 `style.start_angle_degrees`(-90, 위쪽)에서 시작하며
`style.clockwise`가 `true`(기본값)이면 시계 방향으로 돈다.

`RadarRenderer::new`는 RGBA8·BGRA8 포맷(선형 또는 sRGB)만 받는다. `prepare(&chart, size, scale)`의
논리 크기는 240×240 이상, 배율은 0.5~4다. 프레임의 실제 크기는 `size`에 `scale`을 곱한 값이며
장치의 텍스처 한도 안에 들어야 한다. 포맷·크기·배율이 이 범위를 벗어나거나 차트가
`RadarChart::validate`를 통과하지 못하면 `RadarError::Invalid`를 돌려준다. 화면에서는 `frame.size()`
크기에 같은 포맷, sample count 1인 타깃의 render pass에서 `frame.draw(&mut pass)`를 호출한다.

값 축은 모든 스포크가 공유한다. `value_range: None`이면 보기 좋은 고리 간격을 골라, 0에서 시작해
데이터 최댓값 이상인 첫 고리에서 끝나는 범위를 쓴다. 음수 값이 있으면 가장 작은 값 이하의 고리에서
시작한다. `Some([min, max])`는 범위를 고정하고, `ring_spacing`은 고리 간격을 정한다. 고리는 범위 안에 있는
간격의 정수배이며 다른 축과 같은 `model::tick::LinearGrid`로 고르므로, 반올림 오차로 끝을 살짝 넘은
고리(간격 0.1일 때 0.3)도 그린다. 고리 라벨은 `PlainNumberFormat`의 자동 형식으로 적는다. 범위 아래 값은
중심에, 위 값은 바깥 고리에 놓인다. 스포크별 정규화는 호스트가 미리 계산해 값으로 넣고 스포크
라벨도 그에 맞춘다. 유한하지 않은 값은 그 스포크에서 외곽선을 끊고 채움과 마커를 생략한다.

`style.grid_shape`는 고리를 스포크를 지나는 다각형(`Polygon`) 또는 원(`Circle`)으로
그린다. `grid: false`는 고리와 스포크를 숨긴다. 시리즈마다 채움(`RadarSeriesStyle::fill`
또는 시리즈 색을 `style.fill_opacity`로), `line_width_px`의 외곽선, 꼭짓점 마커(`Circle`,
`Square`, `None`)를 그린다. 모든 시리즈의 채움을 먼저, 그다음 외곽선, 마지막에 마커를
그리므로 나중 시리즈의 채움이 앞 시리즈의 외곽선을 가리지 않는다. `labels`가 기본값 `None`이면
데이터 라벨을 그리지 않고, `Values`이면 모든 꼭짓점에, `Emphasized`이면 호버·선택한 시리즈의
꼭짓점에만 값을 적는다. 라벨은 `label_decimals`와 `value_suffix`를 따른다.

`RadarFrame::hit_test`는 차트 논리 픽셀을 받아 `RadarPick { target, part }`를 돌려준다.
마커를 맞히면 꼭짓점(`RadarTarget { series_id, category_id: Some(..) }`), 외곽선이나 채움을
맞히면 시리즈(`category_id: None`)다. 마커, 외곽선, 내부 순서로 각각 그린 순서의 역순으로
판정한다. 대상을 `hovered` / `selected`에 넣으면 해당 시리즈가 `style.emphasis_brightness`만큼
밝아지고 선택한 꼭짓점의 마커가 커진다. `RadarChart::value(&target)`는 꼭짓점 대상의 값을
돌려준다(시리즈 대상이면 `None`). 브라우저에서는 `FiggyBoundedChart`가 같은 모델을 `"radar"`
문서로 받는다. `step_selection(backwards)`는 시리즈 다각형을 차례로 고르고, 꼭짓점을 선택한
동안에는 그 시리즈의 유한한 꼭짓점을 범주 순서로 고른다
([bounded chart host](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#bounded-chart-host) 참고).

`prepare`는 픽셀에 영향을 주는 모든 필드가 같을 때 프레임을 재사용한다. `active_layer`와 주석 층은
이 비교에 들어가지 않는다. 호버·선택·배경·시리즈 스타일만 바꾸면 라벨 래스터도 재사용하지만,
`labels: Emphasized`이면 호버나 선택이 바뀔 때 라벨을 다시 그린다. 이전 프레임을 포함해 프레임을
그린 command buffer를 모두 제출하거나 버린 뒤 `renderer.end_frame()`을 호출해야 큐 작업이 끝날 때
교체된 프레임의 GPU 메모리가 해제된다. 네이티브 호스트는 장치도 poll한다. 렌더러는 직교 차트
렌더러와 별개인 256 MiB 예산을 쓰며(`set_memory_budget`, `gpu_memory_usage`), `end_frame`을 기다리는
메모리도 예산에 포함된다. 예산을 넘게 되면 `prepare`는 `RadarError::Budget`을 돌려준다.
`clear_cache`는 렌더러가 캐시한 프레임 참조만 놓으며, 호스트가 가진 프레임은 계속 쓸 수 있다.

`export_rgba_async`(네이티브 동기 버전은 `export_rgba`)는 `prepare`를 거쳐 주석 층과 현재 호버·선택까지
그리므로 크기·배율 제한도 `prepare`와 같다. 결과는 straight-alpha RGBA8로 읽어 오며
`renderer::encode_png`로 PNG를 만든다.

### 제목·라벨·범례 선택과 이동

데이터 도형의 `hit_test`와 별도로 `frame.decorations()`가 렌더러에서 측정한 히트맵을 제공한다.
제목, 첫 스포크를 따라 놓인 값 라벨 묶음(`AxisLabel(Side::Left)`), 스포크 끝의 범주 라벨
(`DecorationId::SpokeLabel { category_id }`), 범례, 데이터 라벨이 대상이다. 공통
`Selectable`·`Draggable` 계약을 따르므로 `drag_by(&mut chart.decorations, hit, dx, dy)`로 옮기거나
`chart.decorations.reset(&id)`로 자동 위치로 되돌린 뒤 다시 `prepare`를 호출한다. 그리기 영역을
조정하면 플롯 전체가 옮겨지고 크기가 바뀌며, 바깥 고리는 범주 라벨이 들어갈 자리를 남긴 가장
큰 원이다. 좌표 단위, 요소별 이동 규칙, 캐시·저장 복원 및 웹 연결은
[공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)를 참고한다.

### 주석 층

`chart.annotations`는 `renderer.prepare_annotations(&chart, size, scale)`가 돌려주는
`BoundedAnnotationFrame`이 프레임 주위에 그린다. 텍스트 상자·화살표의 앵커는 바깥 고리를 둘러싼
정사각형의 비율이다. 값 축이 따로 없으므로 데이터 앵커와 기준선·구간은
건너뛰고 `render_warnings()`에 `RenderWarning::AnnotationSkipped { reason: NoValueAxis }`로 보고한다.
`draw_underlay` → `frame.draw` → `draw_overlay` 순서로 같은 타깃에 그리고 히트 맵은
`annotations()`로 얻는다. `draws_underlay()`가 `true`이면 `prepare` 전에
`renderer.set_background(FrameBackground::HostClear)`로 바꾸고 `chart.background`로 패스를 지워야
언더레이가 보인다. 자세한 계약은
[공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer)를 참고한다.
