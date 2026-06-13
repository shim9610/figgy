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

- **Standalone**: `wgpu::SurfaceTarget`이 `HtmlCanvasElement` /
  `OffscreenCanvas`를 받으므로, JS가 `<canvas>`를 넘기면
  `for_window_async`가 surface→adapter→device까지 구성한다.
- **Embed**: 웹 호스트(예: eframe 웹 빌드)가 이미 가진 device/queue를
  `RendererDevice`로 주입 — `try_new`는 동기 함수 그대로 사용 가능.

초기화는 async이므로 JS 이벤트 루프에서 구동한다:

```rust
// wasm-bindgen 스케치 — 저장소에 포함된 코드는 아니고 배선 형태만 보여준다.
#[wasm_bindgen]
pub struct FiggyChart { renderer: WindowedRenderer<'static>, /* chart, view, hitmap, … */ }

#[wasm_bindgen]
impl FiggyChart {
    /// JS: `const chart = await FiggyChart.create(canvas);`
    pub async fn create(canvas: web_sys::HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
        let (w, h) = (canvas.width(), canvas.height());
        let renderer = Renderer::for_window_async(
            wgpu::SurfaceTarget::Canvas(canvas), (w, h), 16 * 1024 * 1024,
        ).await.map_err(|e| JsValue::from_str(&e.to_string()))?;
        // … 컬럼 등록 / Chart / ChartView / HitMap::standard_chart() …
    }
}
```

### 3.2 렌더 루프 — requestAnimationFrame + dirty flag

`requestAnimationFrame` 콜백에서 데스크톱 데모와 동일한 패턴을 돈다:

```rust
if chart.consume_raster_dirty() {
    renderer.refresh_axis_with_selection(&mut view, &chart, rect, &sel_boxes)?;
    let _ = chart.consume_data_dirty();
} else if chart.consume_data_dirty() {
    renderer.update_transform(&view, &chart);
}
renderer.draw(clear, &items)?;
```

dirty가 없으면 프레임 비용이 0에 수렴하므로 rAF 상시 구동도 무방하고,
이벤트 시점에만 rAF를 예약하는 절전형도 그대로 성립한다.

### 3.3 데이터 입력 — 업로드는 이미 제로카피, 경계 횡단만 플랫폼 비용

GPU 풀에 올라가는 데이터는 **항상 f32**다. 업로드 설계의 핵심 불변은
native/wasm 공통이다:

```rust
// column_pool::try_add_column — f64 소스는 &dyn ColumnSource로 "빌려서" 읽고,
// f32 변환쓰기의 목적지가 곧 GPU 업로드 버퍼(mapped staging)다. 중간 버퍼 0개.
let mut view = staging.slice(..).get_mapped_range_mut();
source.write_f32_le_into(&mut view[..]);          // f64 참조 → f32 변환쓰기 1회
enc.copy_buffer_to_buffer(&staging, 0, &pool, offset);  // 이후는 GPU 내부 복사
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

- **`Float32Array` (권장)** — 경계 트래픽이 4 B/elem으로 절반,
  `Column<f32>` 경로는 staging에 **순수 memcpy**. GPU 결과는 어느 쪽이든
  f32 한 번 반올림이라 픽셀은 동일하다.
- **`Float64Array`** — 원본이 f64인 데이터에서 min/max 메타데이터(자동
  핏, 로그 변환)의 f64 정밀도를 유지하고 싶을 때.

마샬링 오버헤드까지 줄이려면 wasm이 버퍼를 할당해 ptr/len을 노출하고
JS가 `new Float32Array(memory.buffer, ptr, len).set(src)`로 직접 채우는
패턴을 쓴다 (경계 복사 1회는 동일, wasm-bindgen 인자 변환만 제거).

```rust
#[wasm_bindgen]
pub fn set_column_f32(&mut self, id: &str, data: &[f32]) {
    // 길이+해시가 기존과 같으면 no-op (재업로드 스킵);
    // 다르면 min/max 1회 스캔 → Column<f32> → add_column (staging memcpy)
}
// JS: chart.set_column_f32("x", xs);   // xs: Float32Array, 업서트
```

### 3.4 이벤트 입력 — 포인터를 모델 정책으로 그대로 전달

선택/드래그/리사이즈 정책(`Selectable`/`Draggable`/`Resizable`/`HitMap`)은
전부 `model`에 있고 model은 wasm에서 무수정으로 동작한다. 호스트가 할
일은 canvas 포인터 이벤트를 픽셀 좌표로 바꿔 넘기는 것뿐이다:

```rust
// pointerdown → on_press(e.offsetX * dpr, e.offsetY * dpr)
pub fn on_press(&mut self, x: f32, y: f32) {
    // ① 선택된 요소의 리사이즈 핸들 우선 → ② hit_test 선택 → 드래그 암
    // (winit 데모 handle_click과 동일 로직 — CpuTextMeasure 주입)
}
pub fn on_move(&mut self, dx: f32, dy: f32) { /* drag_by / resize_by */ }
pub fn on_release(&mut self) { /* 드래그/리사이즈 해제 */ }
```

좌표 변환은 `devicePixelRatio` 곱 (egui 데모의 `pixels_per_point`와 같은
역할). `chart_area`가 캔버스 물리 픽셀 기준이면 끝.

### 3.5 이벤트 출력 — CustomEvent로 프레임워크 중립

선택 변경·드래그 종료 등의 결과는 `CustomEvent`로 dispatch하면 React /
Vue / Svelte가 표준 방식으로 구독한다. `<figgy-chart>` Custom Element로
캔버스+wasm 모듈을 감싸는 것이 가장 이식성 높은 포장이다:

```js
canvas.dispatchEvent(new CustomEvent("figgy-select", { detail: { element: "chart-title" } }));
```

### 3.6 PNG export — async 필수, Blob으로 출력

```rust
pub async fn export_png(&self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
    let bytes = self.renderer
        .export_panel_png_bytes_async(&self.chart, &self.series, scale)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(js_sys::Uint8Array::from(bytes.as_slice()))
}
// JS: const png = await chart.export_png(2.0);
//     const url = URL.createObjectURL(new Blob([png], { type: "image/png" }));
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
chart.set_config(JSON.stringify(cfg));     // → dirty → 다음 frame()에 반영

const series = JSON.parse(chart.get_series());
series[0].render_type.Line.line.line_width = 4.0;
series[0].render_type.Line.line.line_color = { r: 1, g: 0, b: 1, a: 1 };
chart.set_series(JSON.stringify(series));  // GPU 스타일 재빌드 포함
```

`set_config`는 `config_mut()` 경유라 양쪽 dirty가 서고, 다음 `frame()`
이 데코 재래스터 + transform 갱신을 처리한다 — 데스크톱과 동일 규약.
**주의**: 스케일을 바꾸면 `major_spacing` 해석도 바뀐다 (Linear = 데이터
단위, Logarithmic = decade 단위). `set_x_range`류 헬퍼는 자동으로 맞춰
주지만 SSoT 직접 편집은 호출자가 함께 고쳐야 한다.

**전체 JSON 스키마는 [`crates/web/SCHEMA.md`](../web/SCHEMA.md)** —
`Config`/`SeriesConfig` 전 필드의 직렬화 형태, enum 허용 문자열, serde
표현 규칙(externally-tagged enum 등), 편집 시 의미 결합 주의사항을
담는다. 이 문서의 JSON 블록은 Rust 소스에서 생성되며 동기화 테스트
관련 검증은 `cargo test -p model --features serde`로 수행한다.

### 3.9 async 메서드와 객체 잠금 (필독)

wasm_bindgen은 async 메서드(`export_png`)의 **프로미스가 pending인 동안
객체를 잠근다** — 그 사이 같은 객체의 다른 메서드를 부르면 "recursive
use of an object" 예외가 난다. 호스트 규약:

- rAF 루프에서 `requestAnimationFrame(tick)`을 **wasm 호출보다 먼저**
  예약해 예외가 루프를 죽이지 못하게 한다.
- export 동안 `busy` 플래그로 `frame()` / 포인터 호출을 건너뛴다.

`crates/web/index.html`이 이 패턴의 레퍼런스 구현이다.

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
| `figgy.d.ts` | **TypeScript 정의 자동 생성** — API 레퍼런스를 겸한다 |
| `package.json` | npm 호환 메타 |

`crates/web/index.html`이 전체 배선(rAF 루프, DPR 좌표 변환, 포인터
선택/드래그/리사이즈, 프리셋 셀렉터, SSoT 편집, PNG 다운로드,
ResizeObserver)을 담은 동작하는 레퍼런스고,
[`crates/web/SCHEMA.md`](../web/SCHEMA.md)가 SSoT JSON의 전체 스키마
레퍼런스다. 로컬 확인:

```bash
cd crates/web && python -m http.server 8137   # wasm은 file:// 불가
```

`FiggyChart` API 표면 (자세한 시그니처는 `figgy.d.ts`):

| 분류 | 메서드 |
|---|---|
| 수명 | `create(canvas)` *(async)* · `resize(w, h)` · `frame()` · `free()` |
| 폰트 | `register_font(Uint8Array)` → 가족명 배열 (TTF/OTF/TTC). 등록 후 SSoT `font` 가족명이 해석됨 — 등록 폰트가 시스템 폰트보다 우선이라 웹/데스크탑 해석이 동일. 미등록·미해석 가족명은 내장 Liberation Sans 폴백 (CJK 글리프 없음 — 한글은 폰트 등록 필요) |
| 스타일 파라미터 | *(free 함수)* `draw_style_modes()` → 모드 태그 JSON 배열 · `draw_style_param_specs(mode)` → `{key, min, max, default, integer}` JSON 배열. **슬라이더 범위의 단일 진실 원본** — min/max는 권장 범위(SSoT는 그 밖의 값도 수용, 렌더러는 안전 가드만 적용), default는 model의 `Default` 구현과 테스트로 고정. 호스트는 이걸로 스타일 UI를 자동 생성하고 범위를 하드코딩하지 말 것 |
| 컬럼 등록/해제 | `set_column_f32(id, Float32Array)` *(업서트)* · `remove_column(id)` |
| 시리즈 등록/해제 | `add_line_series(id, x, y, width, label)` *(업서트)* · `remove_series(id)` |
| 범례 | `set_series_label(id, label)` — `'\n'` 줄바꿈·유니코드 첨자 지원, 빈 문자열 = 해당 행 제거. `set_series` / `apply_color_cycle` 은 자유 편집된 텍스트를 덮지 않고 인식 가능한 자동 엔트리의 심볼만 갱신한다. 전체 재작성은 `reset_legend_from_series_labels()` 를 명시 호출할 때만 수행한다. 자유 편집은 SSoT `legend.content` 하나의 리치 문서로: 줄바꿈은 `"\n"` 세그먼트, `"\t"` 는 표형 열 구분자, 심볼은 **고정폭 필드 세그먼트**(`field_em` — 어떤 형태든 정확히 2.0 em; 선 마크는 `rule:true`, 점선은 `rule_dash` em 패턴) + 색 오버라이드라 위치·줄배치·폭이 전부 명시적. `content.font` / `content.font_size` / 세그먼트별 오버라이드는 그리기 시점에 그대로 적용 |
| 히트테스트 | `hit_test(x, y)` → 요소 id 문자열 또는 `null` (`"data_area"` · `"axis_bottom"` · `"tick_labels_left"` · `"axis_title_left"` · `"legend"` · `"chart_title"` …). 선택 상태 무변경 — 렌더러 자체 레이아웃이 답하므로 호스트가 박스 위치를 복제할 필요 없음 |
| 범위 | `auto_fit_all(pad)` — **등록된 전 시리즈** x/y 합집합에 4방 균일 비율 마진(`0.0` = 딱 맞춤, `0.05` = 5%). **에러바 시리즈는 막대 전체 범위(`값−err_lo … 값+err_hi`, GPU와 동일 산술)가 합집합에 포함**되어 캡이 잘리지 않음 — 쌍별 패스는 (시리즈, 데이터) 조합당 1회 계산·캐싱. 범위 끝 라운딩 없음 — 틱은 범위 안 nice 값에 자동으로 떨어지므로 호스트가 범위를 재가공하지 말 것 · `auto_fit_x/y(col, pad)` (단일 컬럼, 에러바 미반영) · `load_demo()` *(멱등)* |
| SSoT I/O | `get_config()` / `set_config(json)` · `get_series()` / `set_series(json)` |
| 프리셋 | `apply_axis_preset(AxisPreset)` · `apply_color_cycle(ColorCycle)` · `color_cycle_css(cycle)` |
| 상호작용 | `on_press(x, y)` · `on_move(dx, dy)` · `on_release()` · `has_selection()` |
| 출력 | `export_png(scale)` *(async → Uint8Array)* |
| 타이틀 | `set_title` · `set_x_title` · `set_y_title` |

### 등록/해제 모델 — 메모리는 내부 자동 관리

차트는 캔버스당 인스턴스 하나를 두고, 내용은 id 기반 등록/해제로
관리한다. 풀 내부(용량 통계·defrag 정책·핸들)는 노출하지 않는다:

- **`set_column_f32(id, data)` 는 업서트**: 새 id는 업로드, 같은 id에
  **동일 데이터(길이 + 64bit 콘텐츠 해시)는 재업로드 없이 무시** — 호스트
  가 전체 데이터셋을 매번 던져도 변한 컬럼만 GPU에 올라간다. 같은 id의
  다른 데이터는 교체(이전 영역 해제 + 재업로드).
- 업로드 시 auto-fit 용 스칼라 통계(min/max/최소 양수)가 캐싱된다. 점선
  호장(arc-length) 위상 같은 per-point 지오메트리는 GPU 컴퓨트 스캔
  (`line_arc.wgsl`)이 풀 데이터에서 직접 계산한다.
- **에러바 시리즈의 zero-fill 컬럼은 자동 공급**: 한쪽 방향만 쓰는
  errorbar 변종(`ScatterErrorbarY` 등)이 `set_series`로 들어오면, 미사용
  차원에 바인딩되는 내부 `"__zero"` 컬럼을 래퍼가 알아서 등록·확장한다.
  호스트가 이 컨벤션을 알 필요 없음.
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
