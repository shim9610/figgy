# WebAssembly 빌드와 웹 I/O 가이드

`model`/`renderer` 두 crate 모두 `wasm32-unknown-unknown`으로 컴파일된다.
이 문서는 ① 무엇이 어떻게 타겟별로 갈리는지, ② 브라우저에서 다른 웹
컴포넌트와 I/O를 어떻게 이어야 하는지를 정리한다.

확인 명령 (워크스페이스 루트):

```bash
rustup target add wasm32-unknown-unknown
cargo check -p model    --target wasm32-unknown-unknown
cargo check -p renderer --target wasm32-unknown-unknown
```

## 1. 왜 컴파일되는가 — 의존성 구성

| 레이어 | 구성 | wasm |
|---|---|---|
| `model` | 의존성 0 (순수 Rust) | ✅ 무조건 |
| CPU 라스터 (축/라벨/텍스트) | `tiny-skia` + `fontdb` + `swash` — 전부 순수 Rust | ✅ |
| GPU | `wgpu` 27 — 웹에서는 WebGPU 백엔드 | ✅ |
| 블로킹 실행기 | `pollster` — **native 전용 타겟 의존성** | ❌ 컴파일 제외 |

skia-safe는 `wasm32-unknown-emscripten`만 지원해 wasm-bindgen 생태계
(`wasm32-unknown-unknown`)와 혼용이 불가능했고, 그래서 라스터 백엔드를
순수 Rust 스택으로 교체했다. 폰트는 번들 Liberation Sans 4종이 항상
포함되므로 웹에서도 텍스트 렌더가 보장된다. fontdb의 **시스템 폰트
스캔은 native 전용**이지만, `register_font(Uint8Array)` 로 TTF/OTF를
런타임 등록하면 웹에서도 가족명이 해석된다 (등록 폰트 > 시스템 폰트 >
번들 폴백 순).

## 2. 타겟 게이트 — 동기 API는 native 전용, async는 어디서나

웹어셈블리의 메인 스레드는 블로킹이 불가능하다(JS 이벤트 루프와 같은
스레드). 그래서 블로킹 편의 함수들은 `#[cfg(not(target_arch =
"wasm32"))]`로 게이트했고, 같은 일을 하는 async 변형이 모든 타겟에서
제공된다. **수동 feature flag가 아니라 타겟 cfg를 쓴 이유**: 타겟 자체가
플래그라서 "플래그 켜는 걸 잊은 wasm 빌드"가 성립할 수 없다 (wgpu/egui
생태계의 표준 관행).

| 블로킹 (native 전용) | async (모든 타겟) | 내용 |
|---|---|---|
| `Renderer::for_window` | `Renderer::for_window_async` | surface + adapter + device 셋업 |
| `data_render::request_adapter` | `request_adapter_async` | |
| `data_render::request_adapter_for_surface` | `request_adapter_for_surface_async` | |
| `data_render::request_device` | `request_device_async` | |
| `Renderer::export_panel_rgba` | `export_panel_rgba_async` | GPU→CPU readback |
| `Renderer::export_panel_png_bytes` | `export_panel_png_bytes_async` | |
| — | `Renderer::wait_idle` | 웹에서는 no-op (브라우저가 디바이스 폴링) |

블로킹 버전은 전부 `pollster::block_on(async 버전)` 한 줄 래퍼라 구현은
하나다. export의 readback은 `map_async` 완료를 `futures_channel::oneshot`
으로 await하며, native에서는 `device.poll(Wait)`을 인라인 호출해 즉시
resolve되고 웹에서는 await가 JS 이벤트 루프에 양보한다.

**임베드 경로(`Renderer::try_new`)는 원래 블로킹이 없다** — 호스트가
device/queue를 만들어 `RendererDevice`로 주입하는 구조라서, 웹 호스트가
async로 디바이스를 만든 뒤 넘기면 데스크톱과 동일하게 동작한다.

## 3. 웹 I/O 아키텍처

일반 웹 호스트의 public API는 `crates/web/figgy-chart.js`가 등록하는
`<figgy-chart>` Custom Element다. 이 facade가 내부 `<canvas>` 생성,
wasm async init/create, `ready` promise와 `figgy-ready` event,
`requestAnimationFrame` 루프, export 중 busy gate, `ResizeObserver`,
현재 `devicePixelRatio` 기반 backing-store resize, pointer 좌표 변환,
`CustomEvent` dispatch를 맡는다. Raw wasm `FiggyChart` class는 이 facade가
쓰는 low-level kernel이며, 브라우저 수명주기를 직접 소유하려는 advanced
host만 직접 호출한다.

`ready`는 element의 **연결 세대별 Promise**다. 준비되기 전에 disconnect하거나
`free()`하면 그 세대의 Promise는 `AbortError`로 종료되고 다음 연결용 pending
Promise가 설치된다. `free()`는 terminal teardown이므로 다시 DOM에 연결하기
전까지 새 `ready`는 pending이며, 같은 비활성 element에 반복 호출해도 세대나
Promise가 다시 바뀌지 않는다.

```
JS / 웹 프레임워크                      wasm (figgy)
┌──────────────────────┐             ┌─────────────────────────────┐
│ <figgy-chart>         │  canvas     │ Surface ← SurfaceTarget      │
│  (Custom Element)     │────────────▶│   ::Canvas(HtmlCanvasElement)│
│                       │             │                             │
│ Float64Array ─────────┼─ 복사 1회 ──▶ ColumnSource → GPU pool      │
│ pointer events ───────┼─ 메서드 ────▶ HitMap / drag_by / resize_by │
│ CustomEvent ◀─────────┼─ 콜백 ──────│ 선택 / 드래그 / 리사이즈 결과 │
│ Blob 다운로드 ◀───────┼─ async ─────│ export_png_bytes_async       │
└──────────────────────┘             └─────────────────────────────┘
```

### 3.1 그리기 표면 — 데스크톱과 같은 두 경로

- **Standalone facade (권장)**: JS가 `<figgy-chart>`를 배치하면 facade가
  shadow DOM 내부 canvas를 만들고 raw `FiggyChart.create(canvas)`를 async로
  호출한다. host는 `await element.ready` 또는 `figgy-ready` event 이후
  proxy 메서드(`register_column_f32`, `update_register_column_f32`,
  `set_series`, `export_png` 등)를 호출한다.
- **Raw kernel (advanced)**: `wgpu::SurfaceTarget`이 `HtmlCanvasElement` /
  `OffscreenCanvas`를 받으므로, 직접 canvas를 넘기면 `for_window_async`가
  surface→adapter→device까지 구성한다. 이 경로에서는 host가 rAF, DPR
  resize, pointer mapping, busy gate를 전부 직접 지켜야 한다.
- **Embed**: 웹 호스트(예: eframe 웹 빌드)가 이미 가진 device/queue를
  `RendererDevice`로 주입 — `try_new`는 동기 함수 그대로 사용 가능.

Raw kernel 초기화는 async이므로 JS 이벤트 루프에서 구동한다:

```rust
// wasm-bindgen 스케치 — 저장소에 포함된 코드는 아니고 배선 형태만 보여준다.
#[wasm_bindgen]
pub struct FiggyChart {
    renderer: WindowedRenderer<'static>,
    chart_id: ChartId,
    /* view, derived caches, hitmap, … */
}

#[wasm_bindgen]
impl FiggyChart {
    /// JS: `const chart = await FiggyChart.create(canvas);`
    pub async fn create(canvas: web_sys::HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
        let (w, h) = (canvas.width(), canvas.height());
        let renderer = Renderer::for_window_async(
            wgpu::SurfaceTarget::Canvas(canvas), (w, h), 16 * 1024 * 1024,
        ).await.map_err(|e| JsValue::from_str(&e.to_string()))?;
        // … 컬럼 등록 / renderer.register_chart(config, series)
        //   / ChartId / ChartView / HitMap::standard_chart() …
    }
}
```

### 3.2 렌더 루프 — requestAnimationFrame + renderer stamp

`<figgy-chart>` facade가 `requestAnimationFrame` 콜백에서 데스크톱 데모와
동일한 패턴을 돈다. raw kernel을 직접 쓰는 advanced host는 같은 루프를
직접 구현해야 한다. 아래는 실제 `frame()`의 상태 전이만 줄인 의사 코드다:

```rust
renderer.sync_external_invalidations()?; // process-global font registration
let stamp = renderer.chart_render_stamp(chart_id)?;
let renderer_dirty = stamp.needs_draw_since(last_presented_stamp.as_ref());
let raster_dirty = stamp.needs_raster_since(last_presented_stamp.as_ref());

match frame_decision(
    renderer_dirty,
    raster_dirty,
    view_dirty,
    redraw_pending,
    needs_defrag,
) {
    Clean => return Ok(()),
    MaintenanceOnly => {
        process_pending_defrag()?; // surface acquire/draw 없음
        return Ok(());
    }
    Draw { refresh_raster } => {
        ensure_internal_render_columns()?;
        process_pending_defrag()?;
        let stamp = renderer.chart_render_stamp(chart_id)?;
        if refresh_raster {
            renderer.refresh_axis_with_selection(
                &mut view,
                &display_chart,
                rect,
                &sel_boxes,
            )?;
        }
        renderer.draw(clear, &items)?;

        // submit/present 성공 뒤에만 onscreen 상태를 전진시킨다.
        view_dirty = false;
        redraw_pending = false;
        last_presented_stamp = Some(stamp);
    }
}
```

`WindowedRenderer::draw`는 `Renderer::prepare`(`&mut` — pipeline 준비,
transform uniform write, arc-length compute dispatch)와
`Renderer::paint_prepared`(`&self` — 순수 기록)를 한 `&mut self` 아래
연달아 실행하는 원샷 facade다. wasm 래퍼처럼 렌더러를 단독 소유하는
호스트에는 이 facade가 자연스럽고, paint 콜백이 공유 참조만 주는
호스트(egui/iced embed)는 두 단계를 분리 호출한다 — 자세한 계약은
데스크톱 README의 통합 패턴 절 참조.

clean rAF에도 facade의 다음 콜백 예약, DPR 비교, wasm 상태 확인은 남지만
GPU column 준비, surface acquire, draw/submit/present는 전부 생략한다.
실패한 refresh/draw는 last-presented stamp와 host flag를 전진시키지 않아
다음 rAF에서 재시도한다.
이 최적화는 이전 canvas가 그대로 유효한 프레임만 건너뛰며, 원본 데이터의
sampling·LOD·decimation이나 시간 기반 프레임 누락은 수행하지 않는다.

### 3.3 데이터 입력 — 명시적 register/update와 f32 물리 lane

GPU 풀의 물리 lane은 **항상 f32**다. 일반 scalar column은 logical value당
f32 lane 하나, `Float64Array`/`HiLoColumnSource` 경로는 `(hi: f32, lo: f32)`
lane 두 개를 사용한다. 즉 shader의 native f64가 아니라 두 f32의 합으로 큰
절대값에서 작은 delta를 보존한다. 업로드 설계의 핵심 불변은 native/wasm
공통이다:

```rust
// scalar: logical value → one f32 lane
let mut view = staging.slice(..).get_mapped_range_mut();
source.write_f32_le_into(&mut view[..]);
enc.copy_buffer_to_buffer(&staging, 0, &pool, offset);  // 이후는 GPU 내부 복사

// hi/lo: logical f64 value → two f32 lanes in the same mapped staging buffer
source.write_f32_pair_le_into(&mut view[..]);
```

즉 "f64의 소유권/참조만 받아 변환 결과가 업로드 버퍼에 직접 쓰이는가"는
**그렇다** — 데스크톱에서는 이것이 전부다.

wasm에서 추가되는 비용은 변환이 아니라 **메모리 도메인 횡단**이며, 위
구조 바깥의 플랫폼 사정이다:

1. **JS 출발 데이터에 한해** JS 힙 → wasm 선형 메모리 복사 1회. wasm
   안에서 생성·fetch된 데이터라면 이 복사는 없다 (native와 동일해짐).
2. wgpu 웹 백엔드 내부: wasm은 JS `ArrayBuffer`를 `&mut [u8]`로 직접
   가리킬 수 없으므로, `get_mapped_range_mut`는 wasm 쪽 그림자 버퍼를
   내주고 unmap 시 WebGPU의 실제 mapped range로 동기화한다 (wgpu가
   내부 처리하는 1홉).

경계 타입 선택:

- **`Float32Array` (일반 좌표 권장)** — 경계 트래픽 4 B/elem,
  `Column<f32>` 경로는 staging에 **순수 memcpy**.
- **`Float64Array` (큰 절대 좌표)** — 경계 트래픽과 GPU 저장은 8 B/elem.
  min/max 메타데이터뿐 아니라 GPU vertex 계산도 hi/lo 두 f32 lane을 사용해
  timestamp 크기의 절대값에서 sub-f32 delta를 보존한다.

마샬링 오버헤드까지 줄이려면 wasm이 버퍼를 할당해 ptr/len을 노출하고
JS가 `new Float32Array(memory.buffer, ptr, len).set(src)`로 직접 채우는
패턴을 쓴다 (경계 복사 1회는 동일, wasm-bindgen 인자 변환만 제거).

공개 API는 등록과 교체를 구분한다:

```js
chart.register_column_f32("x", xs);          // 새 id만; 기존 id면 오류
chart.update_register_column_f32("x", next); // 기존 id만; 없으면 오류

chart.register_column_f64("time", times);          // Float64Array → hi/lo
chart.update_register_column_f64("time", nextTimes);
```

빈 배열은 거부한다. 승인된 `update_register_*` 호출은 같은 내용이더라도
명시적 교체 요청이므로 매번 failure-atomic upload를 수행한다. hash-only
동일성 판정이나 묵시적 no-op은 없다. `set_series`는 등록된 column id 중
무엇을 그릴지만 바꾸며 column upload를 수행하지 않는다.

### 3.4 이벤트 입력 — 포인터를 모델 정책으로 그대로 전달

선택/드래그/리사이즈 정책(`Selectable`/`Draggable`/`Resizable`/`HitMap`)은
전부 `model`에 있고 model은 wasm에서 무수정으로 동작한다. 일반 host는
`<figgy-chart>` facade가 변환한 pointer event를 쓰면 된다. raw kernel을
직접 쓰는 경우에만 canvas 포인터 이벤트를 픽셀 좌표로 바꿔 넘긴다:

```js
const rect = canvas.getBoundingClientRect();
const sx = canvas.width / Math.max(1, rect.width);
const sy = canvas.height / Math.max(1, rect.height);
const x = (event.clientX - rect.left) * sx;
const y = (event.clientY - rect.top) * sy;
const selected = kernel.on_press(x, y); // Rust Result<bool, JsValue>: 오류는 throw
kernel.on_move(x - lastX, y - lastY);   // Rust Result<(), JsValue>
kernel.on_release();                    // infallible state clear
```

facade는 매 event에서 canvas CSS rect와 backing-store 크기의 비율을 사용해
physical pixel 좌표를 계산한다. `FiggyChart` kernel은 저장된 logical
`chart_area`를 현재 surface에 uniform scale + letterbox로 맞춰 그리고,
drag/resize delta는 내부에서 logical document 좌표로 되돌린다. 따라서
브라우저 viewport resize는 preview zoom이며, Export 문서 크기나 폰트
크기를 바꾸지 않는다.

### 3.5 이벤트 출력 — CustomEvent로 프레임워크 중립

선택 변경·드래그 종료 등의 결과는 facade가 `CustomEvent`로 dispatch하므로
React / Vue / Svelte가 표준 방식으로 구독한다. 이벤트는 custom element에서
`bubbles: true`, `composed: true`로 나간다:

```js
chartEl.addEventListener("figgy-select", (e) => {
  console.log(e.detail.selected);
});
```

### 3.6 PNG export — async 필수, `Uint8Array` 반환

```rust
pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
    self.ensure_zero_column_for_render()?;
    let export_chart = Chart::new(
        self.renderer
            .chart_config(self.chart_id)
            .map_err(js_err)?
            .clone(),
    );
    let series = self
        .renderer
        .chart_series(self.chart_id)
        .map_err(js_err)?
        .to_vec();
    let bytes = self.renderer
        .export_panel_png_bytes_with_clear_async(
            &export_chart,
            &series,
            scale,
            self.clear_color,
        )
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(js_sys::Uint8Array::from(bytes.as_slice()))
}
// JS: const png = await chart.export_png(2.0);
// 필요할 때 host가 new Blob([png], { type: "image/png" })로 변환한다.
```

블로킹 `export_panel_png_bytes`는 웹에 존재하지 않는다(컴파일 제외) —
실수로 메인 스레드를 데드락시킬 방법 자체가 없다.

### 3.7 프리셋 — fieldless enum 그대로 노출

`model::AxisPreset`(축 프레임 5종)과 `model::ColorCycle`(색 로테이션
5종)은 **fieldless enum**이라 wasm_bindgen이 정수 enum으로 그대로
노출한다. 래퍼는 같은 이름의 미러 enum + `From` 변환만 가진다:

```js
chart.apply_axis_preset(AxisPreset.OpenOutward);   // 4축 일괄
chart.apply_color_cycle(ColorCycle.ColorblindSafe); // 시리즈 재색칠 + 범례 동기
color_cycle_css(ColorCycle.Vivid);  // → ["rgb(0 32 240 / 1)", …] 호스트 스와치용
```

### 3.8 SSoT I/O — Config/Series 전체를 JSON으로 라운드트립

옵션 트리(`Config`)와 시리즈 선언(`Vec<SeriesConfig>`)은 GPU 핸들 없는
순수 데이터라서, model의 **`serde` feature**(기본 off — 켜지 않으면
의존성 0 유지)를 켜면 전체가 JSON으로 직렬화된다. 래퍼가 이를
`get_config / set_config / get_series / set_series`로 노출한다:

```js
// 처음엔 auto-fit으로 생산하고, SSoT를 꺼내 자유 편집 후 되돌린다.
const cfg = JSON.parse(chart.get_config());
cfg.left_y.scale = "Logarithmic";          // 스케일
cfg.left_y.major_spacing = 1.0;            //   └ 로그는 decade 단위로 함께!
cfg.left_y.label_style.format = "Power";   // 라벨 포맷 (10ⁿ)
cfg.bottom_x.tick = "Both";                // 틱 모양
cfg.bottom_x.major_tick_length = 12.0;     // 틱 길이
cfg.bottom_x.label_style.color = { r: 0.8, g: 0.1, b: 0.1, a: 1.0 };  // 색
cfg.chart_title.text.font_size = 34;       // 글씨
chart.set_config(JSON.stringify(cfg));     // → renderer revision 갱신 → 다음 frame()에 반영

const series = JSON.parse(chart.get_series());
series[0].render_type.Line.line.line_width = 4.0;
series[0].render_type.Line.line.line_color = { r: 1, g: 0, b: 1, a: 1 };
chart.set_series(JSON.stringify(series));  // GPU 스타일 재빌드 포함
```

`set_config`는 JSON을 검증한 뒤 renderer-owned `Config`를
`set_chart_config(chart_id, config)`로 교체한다. 이 호출이 desired/config/
raster revision을 갱신하므로 다음 `frame()`의 `ChartRenderStamp` 비교가
draw와 raster refresh를 요구한다.
**주의**: 스케일을 바꾸면 `major_spacing` 해석도 바뀐다 (Linear = 데이터
단위, Logarithmic = decade 단위). `set_x_range`류 헬퍼는 자동으로 맞춰
주지만 SSoT 직접 편집은 호출자가 함께 고쳐야 한다.
`AxisOptions.inverted` 역시 별도 wasm 메서드가 아니라 Config JSON 필드이며,
축 라스터·데이터 렌더링·`pick_point`가 같은 반전 mapping을 사용한다.

**전체 JSON 스키마는 [`crates/web/SCHEMA.md`](../web/SCHEMA.md)** —
`Config`/`SeriesConfig` 전 필드의 직렬화 형태, enum 허용 문자열, serde
표현 규칙(externally-tagged enum 등), 편집 시 의미 결합 주의사항을
담는다. 이 문서의 JSON 블록은 Rust 소스에서 생성되며 동기화 테스트
관련 검증은 `cargo test -p model --features serde`로 수행한다.

### 3.9 async 메서드와 객체 잠금 (필독)

wasm_bindgen은 async 메서드(`export_png`)의 **프로미스가 pending인 동안
객체를 잠근다** — 그 사이 같은 객체의 다른 메서드를 부르면 "recursive
use of an object" 예외가 난다. facade는 이 규약을 내부에서 지킨다.
raw kernel 직접 호출 시 host 규약:

- rAF 루프에서 `requestAnimationFrame(tick)`을 **wasm 호출보다 먼저**
  예약해 예외가 루프를 죽이지 못하게 한다.
- export 동안 `busy` 플래그로 `frame()` / 포인터 / resize / proxy 호출을
  모두 건너뛰거나 거부한다.

`crates/web/index.html`은 facade 사용 레퍼런스다. raw kernel 직접 배선은
advanced host가 위 규약을 그대로 복제할 때만 선택한다.

## 4. 제약과 주의사항

- **단일 스레드**: CPU 라스터(축 크롬)는 메인 스레드에서 돈다. 패널 단위
  데코 래스터는 글리프 캐시 적용 후 ~0.4 ms/frame(release, 600×460)이라
  상호작용 중에도 문제없다. 더 큰 작업이 필요해지면 `OffscreenCanvas` +
  Web Worker로 전체를 옮기는 선택지가 있고, wasm 스레드(SharedArrayBuffer)
  를 쓰려면 서버에서 COOP/COEP 헤더(cross-origin isolation)가 필요하다.
- **WebGPU 가용성**: Chrome/Edge 안정판, Firefox 141+, Safari 26+. 구형
  브라우저 대응이 필요하면 wgpu의 `webgl` feature로 WebGL2 폴백을 켤 수
  있다 (이 경우 WebGPU 전용 한계치 차이에 유의).
- **폰트**: 번들 Liberation Sans에는 CJK 글리프가 없다 — 한글 등은
  호스트가 `register_font(Uint8Array)` 로 폰트 파일(TTF/OTF)을 가져와
  등록해야 한다 (등록 후 SSoT `font` 가족명으로 사용; 반환값이 가족명).
  woff2는 fontdb가 파싱하지 못하므로 TTF/OTF를 받을 것.
  **손그림(sketch) 모드는 텍스트 폰트를 자동으로 번들 손글씨 폰트(Comic
  Neue, OFL)로 강제한다** — 별도 등록 불필요. Comic Neue가 글리프를
  갖지 않는 문자(CJK·그리스 등)는 문자 단위로 일반 해석 체인(등록 폰트 →
  Liberation)으로 폴백하므로, 한글 라벨은 sketch 모드에서도 등록해 둔
  CJK 폰트로 그대로 그려진다.
- **pollster 함정**: 직접 wgpu 코드를 추가할 때 wasm에서 `block_on`을
  쓰면 데드락이다. 이 저장소의 규약대로 — 블로킹 변형은
  `#[cfg(not(target_arch = "wasm32"))]`, 본 구현은 async — 를 따를 것.

## 5. 빌드 산출물 — `crates/web` → `pkg/`

래퍼 crate는 `crates/web`(패키지명 `figgy` — 대외 산출물 이름)이고,
릴리즈 빌드는:

```bash
npx wasm-pack build crates/web --release --target web
```

산출물 (`crates/web/pkg/`, 프론트엔드에 통째로 전달):

| 파일 | 내용 |
|---|---|
| `figgy_bg.wasm` | 릴리즈 wasm 본체 (~3.6 MB — wgpu + 번들 폰트 4종 포함) |
| `figgy.js` | ES module 글루 — `import init, { FiggyChart, … }` |
| `figgy.d.ts` | **TypeScript 정의 자동 생성** — raw wasm kernel 시그니처 레퍼런스 |
| `package.json` | npm 호환 메타 |

`crates/web/figgy-chart.js`가 public facade다(`pkg/` 산출물이 아니라 함께
배포하는 JS entry). 내부 canvas, rAF 루프, DPR
좌표 변환, 포인터 선택/드래그/리사이즈, ResizeObserver, ready/event
수명주기, export busy gate를 포함한다. `crates/web/index.html`은 이 facade를
사용하는 동작 레퍼런스고,
[`crates/web/SCHEMA.md`](../web/SCHEMA.md)가 SSoT JSON의 전체 스키마
레퍼런스다. 로컬 확인:

```bash
cd crates/web && python -m http.server 8137   # wasm은 file:// 불가
```

`<figgy-chart>` facade API 표면:

| 분류 | 메서드 |
|---|---|
| 수명 | `<figgy-chart>` element · `ready` promise · `figgy-ready` / `figgy-error` / `figgy-select` / `figgy-drag` / `figgy-release` / `figgy-resize` events · `free()` |
| 폰트 | `register_font(Uint8Array)` → 가족명 배열 (TTF/OTF/TTC). 등록 후 SSoT `font` 가족명이 해석됨 — 등록 폰트가 시스템 폰트보다 우선이라 웹/데스크탑 해석이 동일. byte-for-byte 동일 파일의 재등록은 저장소와 font generation을 늘리지 않는 멱등 동작이며, resolved face backing도 face id별로 재사용한다. 미등록·미해석 가족명은 내장 Liberation Sans 폴백 (CJK 글리프 없음 — 한글은 폰트 등록 필요) |
| 스타일 파라미터 | *(free 함수)* `draw_style_modes()` → 모드 태그 JSON 배열 · `draw_style_param_specs(mode)` → `{key, min, max, default, integer}` JSON 배열. **슬라이더 범위의 단일 진실 원본** — min/max는 권장 범위(SSoT는 그 밖의 값도 수용, 렌더러는 안전 가드만 적용), default는 model의 `Default` 구현과 테스트로 고정. 호스트는 이걸로 스타일 UI를 자동 생성하고 범위를 하드코딩하지 말 것 |
| 컬럼 등록/갱신/해제 | `register_column_f32/f64(id, TypedArray)` *(새 id만)* · `update_register_column_f32/f64(id, TypedArray)` *(기존 id만, 승인된 호출은 항상 upload)* · `remove_column(id)` |
| 시리즈 등록/해제 | `add_line_series(id, x, y, width, label)` *(업서트)* · `remove_series(id)` |
| 범례 | `set_series_label(id, label)` — `'\n'` 줄바꿈·유니코드 첨자 지원, 빈 문자열 = 해당 행 제거. `set_series` / `apply_color_cycle` 은 자유 편집된 텍스트를 덮지 않고 인식 가능한 자동 엔트리의 심볼만 갱신한다. 전체 재작성은 `reset_legend_from_series_labels()` 를 명시 호출할 때만 수행한다. 자유 편집은 SSoT `legend.content` 하나의 리치 문서로: 줄바꿈은 `"\n"` 세그먼트, `"\t"` 는 표형 열 구분자, 심볼은 **고정폭 필드 세그먼트**(`field_em` — 어떤 형태든 정확히 2.0 em; 선 마크는 `rule:true`, 점선은 `rule_dash` em 패턴) + 색 오버라이드라 위치·줄배치·폭이 전부 명시적. `content.font` / `content.font_size` / 세그먼트별 오버라이드는 그리기 시점에 그대로 적용 |
| 히트테스트 | `hit_test(x, y)` → 요소 id 문자열 또는 `null` (`"data_area"` · `"axis_bottom"` · `"tick_labels_left"` · `"axis_title_left"` · `"legend"` · `"chart_title"` …). `pick_point(x, y, max_distance_px)` → `Promise<{ source_id: string \| null, series_id, point_index, distance_px } \| null>`; point/scatter는 실제 marker 크기(스타일 매핑 포함)를 기준으로, line 계열은 stroke 근처 클릭을 해당 segment의 가까운 endpoint 데이터 점으로 스냅한다. errorbar stem/cap 자체는 pick target이 아니다. 좌표가 필요하면 host가 `point_index`로 자신이 등록한 원본 column을 조회한다. 선택 상태 무변경 — 렌더러 자체 레이아웃이 답하므로 호스트가 박스 위치를 복제할 필요 없음 |
| 범위 | `auto_fit_all(pad)` *(Promise)* — **등록된 전 시리즈의 원본 primitive data domain** x/y 합집합에 4방 균일 비율 마진(`0.0` = 딱 맞춤, `0.05` = 5%). 원본 GPU 컬럼을 축약·샘플링 없이 전수 reduce한다. line-only는 유효한 인접 segment의 endpoint, scatter-bearing 시리즈는 유한한 x/y 쌍, errorbar는 실제 six-column 공통 행에서 현재 renderer와 같은 방향 활성 조건을 통과한 `값−err_lo … 값+err_hi` endpoint를 포함한다. 짧은 error 컬럼은 errorbar endpoint 범위만 제한하며 그 뒤의 유효한 base line/scatter 행을 자르지 않는다. 이 primitive 조건에서 탈락한 non-finite 행과 고립된 line point는 제외하고 normalized mode와 역할별 column revision으로 결과를 캐싱한다. readback 결과는 다음 `frame()`에서 token이 여전히 current일 때 renderer-owned Config에 commit된 뒤 Promise가 resolve된다. facade는 지속 rAF로 이를 처리하며 raw kernel host는 pending 동안 `frame()`을 계속 호출해야 한다. 범위 끝 라운딩 없음 — 틱은 범위 안 nice 값에 자동으로 떨어지므로 호스트가 범위를 재가공하지 말 것 · `auto_fit_x/y(col, pad)` (단일 컬럼 upload metadata, 에러바 미반영) · `load_demo()` *(멱등)* |
| 피킹 기준 | 최종 스타일/래스터 픽셀이 아니라 원본 시리즈 primitive를 판정한다. scatter는 원본 데이터 점 위치와 설정된 marker hit 반경을 사용하고, line은 인접한 원본 데이터 점 사이의 직선 segment를 검사해 가까운 endpoint로 스냅한다. dash 공백, square-cap 래스터 모서리, sketch 등 장식용 변형은 pick 경로를 바꾸지 않는다. |
| SSoT I/O | `get_config()` / `set_config(json)` · `get_series()` / `set_series(json)` |
| 프리셋 | `apply_axis_preset(AxisPreset)` · `apply_color_cycle(ColorCycle)` · `color_cycle_css(cycle)` |
| 상호작용 | facade가 pointer event를 내부 처리. Advanced proxy: `on_press(x, y)` · `on_move(dx, dy)` · `on_release()` · `has_selection()` |
| 출력 | `export_png(scale)` *(async → Uint8Array)* |
| 타이틀 | `set_title` · `set_x_title` · `set_y_title` |

Per-point style mapping is configured only through `set_series(json)` / `get_series()`.
Precise scatter uses `point_style_table` / `point_style_index_column` / `point_style_overrides`;
precise errorbars use `error_bar_style_table` / `error_bar_style_index_column` /
`error_bar_style_overrides`. These fields do not add separate wasm methods, and styled draw
modes ignore the mappings.

Advanced escape hatch: `element.kernel`은 raw wasm `FiggyChart`를 반환한다.
이 경로는 facade의 busy gate와 browser lifecycle 캡슐화를 우회하므로, 일반
host 계약이 아니라 디버깅/특수 embed용이다.

### 등록/해제 모델 — 메모리는 내부 자동 관리

차트는 캔버스당 인스턴스 하나를 두고, 내용은 id 기반 등록/해제로
관리한다. 풀 내부(용량 통계·defrag 정책·핸들)는 노출하지 않는다:

- **`register_column_f32/f64(id, data)` 는 새 id 전용**: 기존 id면 오류.
- **`update_register_column_f32/f64(id, data)` 는 기존 id 전용**: 없는 id면
  오류. 호출 자체가 내용 교체 의사이므로 승인된 호출은 같은 값이어도 매번
  failure-atomic upload를 수행한다. hash-only no-op 판정은 사용하지 않는다.
- **`set_series(json)`은 column id 지정만 변경**: 등록/교체 upload를
  수행하지 않는다.
- 업로드 시 auto-fit 용 스칼라 통계(min/max/최소 양수)가 캐싱된다. 점선
  호장(arc-length) 위상 같은 per-point 지오메트리는 GPU 컴퓨트 스캔
  (`line_arc.wgsl`)이 풀 데이터에서 직접 계산한다.
- **에러바 시리즈의 zero-fill 컬럼은 render 준비가 소유**: `"__zero"`는
  public mutation이 금지된 reserved id다. 한쪽 방향만 쓰는 errorbar 변종의
  실제 draw/export 직전에만 필요한 길이로 renderer-owned filler를
  생성·확장한다. `set_series` 자체는 어떤 column도 upload하지 않는다.
- **`remove_column(id)`** 은 그 컬럼을 참조하는 시리즈까지 자동으로 내려서,
  해제된 데이터를 가리키는 프레임이 존재할 수 없다. 자동 관리 범례에서는
  대응 행도 제거하고, `set_config` 로 자유 편집된 범례에서는 사용자 텍스트를
  보존한 채 남은 인식 가능 심볼만 갱신한다.
- **defrag 자동**: 제거/교체로 생긴 풀 구멍은 다음 `frame()` 시작에서
  1회로 통합 압축되고(GPU 내부 복사), 연속 교체 중 일시 단편화는
  `OnAllocFailure` 정책이 흡수한다.
- **`add_line_series`도 series_id 업서트** — 기존 id는 제자리 교체(색
  유지), 새 id는 색 로테이션의 다음 색. 빈 label 로 기존 id를 업서트해도
  기존 범례 텍스트는 제거되지 않는다. 비어 있지 않은 label 은 해당 행의
  텍스트만 갱신한다.
- **인스턴스 해제 = `free()`** (wasm-bindgen 자동 생성): drop 체인이 풀
  버퍼·파이프라인·텍스처·surface까지 내린다. GC FinalizationRegistry
  폴백이 있지만 비결정적이므로 **SPA 언마운트 시 `free()` 명시 호출**이
  규약이다.

`wasm-opt`는 비활성 상태다 (wasm-pack 번들 binaryen이 최신 rustc 출력
기능에서 크래시 — `crates/web/Cargo.toml`의 메타데이터 참고). Rust
릴리즈 최적화는 적용되어 있으며, 사이즈 추가 절감이 필요해지면 최신
binaryen으로 다시 켠다.
