<a id="webassembly-빌드와-웹-io-가이드"></a>

# WebAssembly 지원과 브라우저용 figgy

Rust 렌더러의 WASM/WebGPU 지원과 figgy의 브라우저용 제품인 `crates/web` 래퍼를 구분한다.
아래 `FiggyChart`, `<figgy-chart>`와 JavaScript 호출 방법은 이 래퍼의 계약이다.
Rust 라이브러리가 모든 차트를 JavaScript API로 제공한다는 뜻은 아니다.
`FiggyChart`와 `<figgy-chart>`는 직교 좌표 차트용이며, `Config.polar`를 켠 극좌표 차트도 그린다. 극좌표에서는 `x_column` 값을 `angle_unit` 단위의 각도로 보고 `zero_angle_degrees`에서부터 재므로, `bottom_x`의 `min` / `max`는 각도 매핑을 바꾸지 않고 그릴 각도 눈금·스포크·라벨만 정한다([극좌표 안내](POLAR.md) 참고). 범주형 막대 차트·박스플롯·레이더 차트도 `FiggyChart`가 그리며(`set_categorical_document`, `set_boxplot_document`, `set_radar_document`), 원형 차트는 같은 패키지의 저수준 클래스 `FiggyBoundedChart`로 제공하며, `<figgy-chart>`는 이 클래스를 감싸지 않는다([Radial chart host](#bounded-chart-host) 참고).
간단한 빌드·실행 방법은 [브라우저용 figgy 안내](https://github.com/shim9610/figgy/blob/master/crates/web/README.md)를 참고한다.

The [streaming contract](#streaming-contract) and the [radial chart host](#bounded-chart-host) sections of this guide are in English.

이 문서는 브라우저 패키지 `figgy` 0.15.1과 `figgy-renderer` 0.18.1 / `figgy-model` 0.12.1을 기준으로 한다. `figgy`는 crates.io에 배포하지 않으므로 이 저장소에서 빌드한다. Rust 호스트는 crates.io의 렌더러를 의존성으로 지정한다. 웹 래퍼처럼 설정을 JSON으로 주고받으려면 `serde` 기능을 켠다.

```toml
renderer = { package = "figgy-renderer", version = "0.18.1", features = ["serde"] }
```

렌더러 없이 모델 타입만 쓰는 크레이트는 `model = { package = "figgy-model", version = "0.12.1" }`을 지정한다. 로컬 체크아웃을 쓰려면 `version` 대신 `path`로 `crates/renderer`를 가리킨다. 0.17.0용으로 작성한 Rust 코드는 [마이그레이션 안내](https://github.com/shim9610/figgy/blob/master/CHANGELOG.md#migrating-from-0170)에 따라 고친다.

`model`과 `renderer`는 모두 `wasm32-unknown-unknown`으로 빌드할 수 있다. 이 문서는 네이티브와 웹의 차이, 브라우저 초기화, 데이터 입력, 이벤트 처리와 이미지 출력을 설명한다.

`rust-toolchain.toml`에 검증한 개발 환경인 **Rust 1.99.0**을 고정했다. Rust 패키지의 지원 하한도 1.99이며, 더 낮은 버전은 검증하지 않았다. wasm-pack은 **0.15.0**을 사용하며, 이 저장소를 빌드할 때는 커밋된 `Cargo.lock`을 따른다.

워크스페이스 루트에서 다음 명령으로 확인할 수 있다.

```bash
rustup target add wasm32-unknown-unknown
cargo check --locked -p figgy-model    --target wasm32-unknown-unknown
cargo check --locked -p figgy-renderer --target wasm32-unknown-unknown
```

<a id="1-왜-컴파일되는가--의존성-구성"></a>

## 1. WebAssembly를 지원하는 의존성 구성

| 레이어 | 구성 | wasm |
|---|---|---|
| `model` | 기본 의존성 없음; 선택적 `serde` (순수 Rust) | ✅ |
| CPU 래스터 (축/라벨/텍스트) | `tiny-skia` + `fontdb` + `swash` — 전부 순수 Rust | ✅ |
| GPU | `wgpu` 30 — 웹에서는 WebGPU 백엔드 | ✅ |
| 동기 실행기 | `pollster` — **네이티브 빌드 전용 의존성** | ❌ 컴파일 제외 |

CPU 렌더링에는 순수 Rust로 구현된 tiny-skia·fontdb·swash를 사용한다.

Liberation Sans 네 가지 글꼴을 내장하므로 웹에서도 이 폰트의 문자를 그릴 수 있다. 시스템 폰트 검색은 네이티브에서만 지원한다. 웹에서 다른 폰트를 쓰려면 `register_font(Uint8Array)`로 TTF/OTF/TTC 파일을 등록한다. 폰트는 등록된 폰트, 시스템 폰트, 내장 대체 폰트 순으로 찾는다. 찾은 폰트에 없는 글자는 글자마다 등록 폰트(등록한 순서), 같은 굵기·기울기의 내장 Liberation Sans, 설치된 시스템 폰트(글꼴 이름순, 네이티브 전용) 순으로 그 글자를 그릴 수 있는 폰트를 찾아 그린다. 등록 폰트와 내장 폰트에서 찾는 글자는 웹과 네이티브에서 결과가 같다. 어느 폰트에도 없는 글자만 처음 찾은 폰트의 `.notdef` 상자로 표시한다.

<a id="2-타겟-게이트--동기-api는-native-전용-async는-어디서나"></a>

## 2. 비동기 API와 네이티브 전용 동기 API

웹의 WASM 코드는 JavaScript 이벤트 루프와 같은 메인 스레드에서 실행되므로 동기 대기를 사용할 수 없다. 동기 편의 함수는 `#[cfg(not(target_arch = "wasm32"))]`로 네이티브에서만 빌드하고, 같은 작업을 하는 비동기 함수는 모든 플랫폼에서 제공한다. 별도 기능 플래그를 켤 필요 없이 빌드 대상에 따라 자동으로 선택된다.

| 동기 API(네이티브 전용) | 비동기 API(모든 플랫폼) | 내용 |
|---|---|---|
| `Renderer::for_window` | `Renderer::for_window_async` | surface·adapter·device 초기화 |
| `data_render::request_adapter` | `request_adapter_async` | |
| `data_render::request_adapter_for_surface` | `request_adapter_for_surface_async` | |
| `data_render::request_device` | `request_device_async` | |
| `Renderer::export_panel_rgba` | `export_panel_rgba_async` | GPU→CPU readback |
| `Renderer::export_panel_png_bytes` | `export_panel_png_bytes_async` | |
| `Renderer::export_document_rgba` | `export_document_rgba_async` | 기본 레이어와 모든 오버레이 레이어를 한 이미지로 합성 |
| `Renderer::export_document_png_bytes` | `export_document_png_bytes_async` | |
| — | `Renderer::wait_idle`(동기, 모든 플랫폼) | 네이티브에서는 장치 작업이 끝날 때까지 대기하고, 웹에서는 아무 작업도 하지 않는다 |
| — | `Renderer::wait_submitted_work` / `WindowedRenderer::first_frame_ready` / `warm_up` | 첫 `queue.submit()` 이후 GPU 작업 완료 대기 |

동기 버전은 비동기 함수를 `pollster::block_on`으로 감싼 래퍼다. 이미지 출력의 GPU 결과 읽기는 `map_async` 완료를 `futures_channel::oneshot`으로 기다린다. 네이티브에서는 `device.poll(Wait)`로 완료를 처리하고, 웹에서는 `await`로 JavaScript 이벤트 루프에 실행을 넘긴다.

웹 이미지 출력은 같은 wgpu 장치의 `GPUDevice.pushErrorScope` / `popErrorScope`로 메모리 부족과 내부 오류를 확인한다. 오류 범위가 비동기 함수 전체에 걸쳐 열린 채 남지 않도록 자원 생성·렌더 제출과 각 결과 읽기 제출을 별도의 동기 구간으로 처리한다. 각 구간은 `push → 명령 기록·제출·매핑 요청 → pop 요청`을 마친 뒤 반환된 Promise만 기다린다.

따라서 출력 작업이 취소돼도 다음 호출에 열린 오류 범위가 남지 않는다. 외부 호스트가 같은 장치에 설정한 오류 범위와도 대기 중 순서가 뒤섞이지 않는다. 동기 구간이 일찍 종료되면 RAII 정리 코드가 `popErrorScope()`를 요청한다. 실제 오류의 name·constructor·message는 `FiggyError::GpuResourceAllocationFailed`에 보존하며, 정상 결과인 `null`은 오류로 바꾸지 않는다. 네이티브 출력은 기존 wgpu 메모리 부족 검사 방식을 사용한다.

**기존 장치에 연결하는 `Renderer::try_new`는 동기 대기를 하지 않는다.** 호스트가 장치와 큐를 만든 뒤 `RendererDevice`로 전달한다. 웹에서도 장치를 비동기로 생성한 다음 같은 방식으로 사용할 수 있다.

## 3. 웹 I/O 아키텍처

일반적인 웹 통합에는 `crates/web/figgy-chart.js`가 등록하는 `<figgy-chart>` 사용자 정의 요소를 사용한다. 이 웹 래퍼가 내부 캔버스 생성, WASM 초기화, `ready` Promise와 `figgy-ready` 이벤트, `requestAnimationFrame` 루프, 비동기 호출 직렬화, `ResizeObserver`, DPR에 따른 캔버스 픽셀 크기 조절, 포인터 좌표 변환과 `CustomEvent` 전달을 담당한다.

저수준 WASM 클래스인 `FiggyChart`는 이 래퍼가 사용하는 커널이다. 브라우저의 실행 흐름과 객체 수명을 직접 관리해야 할 때만 호출한다.

`ready`는 DOM에 연결할 때마다 새로 만드는 Promise다. 준비 전에 연결을 끊거나 `free()`를 호출하면 해당 Promise는 `AbortError`로 종료되고 다음 연결을 위한 Promise가 만들어진다. `free()`로 해제한 요소는 다시 연결하기 전까지 준비되지 않는다. 이미 비활성 상태인 요소에 `free()`를 반복 호출해도 연결 세대나 Promise는 바뀌지 않는다.

| API·상태 | 초기화와 수명 관리 |
|---|---|
| 저수준 `FiggyChart` | `create` / `create_with_progress`는 같은 GPUDevice에서 모든 렌더링용 WGSL 진입점을 `createRenderPipelineAsync`로 미리 컴파일하고 임시 JS 파이프라인을 해제한다. 이후 빈 차트의 첫 프레임 완료를 기다린다. 선택적인 렌더링·스타일 자원과 경로 길이·범위 계산·피킹·등고선 컴퓨트 캐시는 처음 필요할 때 만든다. `prewarm_all_with_progress(callback)` / `prewarm_all()`로 이 캐시들을 미리 생성할 수 있다. 사전 준비 메서드와 피킹 준비 순서는 [3.1 캔버스와 장치 초기화](#31-그리기-표면--데스크톱과-같은-두-경로)를 참고한다. |
| 래퍼의 준비 완료 | `web.create / first frame / finished` 진행 알림과 `figgy-ready`를 보낸 뒤 백그라운드에서 피킹을 준비한다. 실패하면 복구 가능한 `figgy-error`로 알린다. 이미 완료된 `ready`와 렌더링 루프는 유지한다. |
| 래퍼의 작업 중 상태 | 연결 세대와 커널을 함께 확인하는 작업 토큰으로 초기화와 상태를 바꾸는 비동기 WASM 호출을 직렬화한다. 두 전체 사전 준비 메서드도 이 경로를 따른다. 작업 중에는 프레임과 입력이 WASM에 접근하지 않고, `get_config` 같은 `<figgy-chart>`의 동기 메서드는 `"figgy chart is busy"` 오류를 던진다. 마지막 크기 변경과 포인터 해제만 보관했다가 작업 종료 후 적용한다. |
| 연결 종료·재연결 | 이전 연결 세대의 토큰을 무효화한다. 실행 중인 작업이 사용하는 커널은 작업이 끝난 뒤 해제한다. 이전 작업의 완료 처리는 새 세대의 토큰이나 커널을 변경하지 않는다. |

```
JS / 웹 프레임워크                      wasm (figgy)
┌──────────────────────┐             ┌─────────────────────────────┐
│ <figgy-chart>         │  canvas     │ Surface ← SurfaceTarget      │
│  (Custom Element)     │────────────▶│   ::Canvas(HtmlCanvasElement)│
│                       │             │                             │
│ Float64Array ─────────┼─ 복사 1회 ──▶ ColumnSource → GPU pool      │
│ pointer events ───────┼─ 메서드 ────▶ HitMap / drag_by / resize_by │
│ CustomEvent ◀─────────┼─ 콜백 ──────│ 선택 / 드래그 / 리사이즈 결과 │
│ Blob 다운로드 ◀───────┼─ async ─────│ export_png(scale)            │
└──────────────────────┘             └─────────────────────────────┘
```

<a id="31-그리기-표면--데스크톱과-같은-두-경로"></a>

### 3.1 캔버스와 장치 초기화

- **웹 래퍼 사용(권장)**: `<figgy-chart>`를 배치하면 래퍼가 shadow DOM 안에 캔버스를 만들고 `FiggyChart.create_with_progress(canvas, callback)`을 비동기로 호출한다. `await element.ready` 또는 `figgy-ready` 이후 `register_column_f32`, `update_register_column_f32`, `set_series`, `export_png` 등의 메서드를 사용할 수 있다.
- **저수준 커널 직접 사용**: `wgpu::SurfaceTarget`은 `HtmlCanvasElement`와 `OffscreenCanvas`를 받는다. 캔버스를 전달하면 `for_window_async`가 surface·adapter·device를 구성한다. 이 경우 rAF 루프, DPR과 크기 변경, 포인터 좌표 변환, 비동기 호출 중 접근 제한은 호스트가 직접 관리해야 한다.
- **기존 렌더러에 통합**: eframe 웹 빌드처럼 호스트가 장치와 큐를 가지고 있다면 `RendererDevice`로 전달한다. `try_new`는 동기 함수로 사용할 수 있다.

저수준 커널 초기화는 JavaScript 이벤트 루프에서 비동기로 실행한다.

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
        let mut renderer = Renderer::for_window_async(
            wgpu::SurfaceTarget::Canvas(canvas), (w, h), 16 * 1024 * 1024,
        ).await.map_err(|e| JsValue::from_str(&e.to_string()))?;
        // picker는 첫 pick 전, 또는 여기서 명시적으로 enable.
        // … 컬럼 등록 …
        let chart_id = renderer.register_chart(config, series).map_err(js_err)?;
        // … ChartView / HitMap::standard_chart() …
    }
}
```

`for_window_async`는 어댑터·장치 생성 이후 파이프라인 준비 단계마다 브라우저에 실행을 넘긴다. `InitEvent`와 WASM의 `requestAnimationFrame`을 사용하므로 초기화 중에도 로딩 표시가 갱신된다. `<figgy-chart>`는 `create_with_progress`의 진행 상황을 `figgy-init-progress` 이벤트의 `{ scope, stage, phase }`로 전달한다. 이 콜백에서는 커널 메서드를 호출하지 않아야 한다. 객체가 아직 없거나 wasm-bindgen이 변경 가능한 참조를 사용 중일 수 있기 때문이다.

`FiggyChart.create` / `create_with_progress`는 빈 차트의 첫 프레임을 제출한 뒤 실제 `GPUQueue.onSubmittedWorkDone()` Promise를 기다린다(`first_frame_ready`). 큐 요청 거부나 장치 소멸은 생성·준비 오류로 전달하며 첫 프레임의 `finished` 이벤트를 보내지 않는다. 성공·실패 모두에서 호출될 수 있는 wgpu 완료 콜백만으로 준비 성공을 판단하지 않는다.

첫 프레임 전에 fullscreen·선·점·오차 막대·막대·행렬, 모든 스타일·매핑, 선택 테두리·데이터 선택 표시·등고선 라벨의 렌더링 진입점을 `createRenderPipelineAsync`로 차례로 준비한다. 임시 JS 파이프라인은 즉시 해제하며 같은 GPUDevice의 셰이더·드라이버 캐시만 준비한다. 생성 과정에서 필요한 기본 wgpu 객체 외의 선택적 렌더링·스타일 캐시와 경로 길이·범위 계산·피킹·등고선 컴퓨트 캐시는 이 단계에서 만들지 않는다. 각 진입점의 started/finished 사이에는 JS 이벤트 루프가 실행되므로 파일 파싱·시트 편집·진행 표시를 계속할 수 있다.

`prewarm_all_with_progress(callback)`은 상주 렌더링에 필요한 자원을 실제 렌더러 캐시에 미리 생성한다. 정밀 모드의 선·점·오차 막대와 스타일 매핑, 점 선택 테두리, 막대·셀·등고선 선택 표시, 히스토그램·히트맵·등고선, 스케치·은하수·별자리, 등고선 라벨, 경로 길이 스캔, 시리즈 범위 계산과 GPU 피킹이 대상이다. 진행 상황은 `{ scope, stage, phase }`로 전달한다. `prewarm_all()`은 콜백 없이 같은 작업을 한다.

준비 중에는 wasm-bindgen이 객체의 변경 가능한 참조를 사용한다. 호스트는 해당 차트 호출을 대기시켜야 하지만 다른 앱 작업까지 막을 필요는 없다. 웹 래퍼의 두 메서드는 연결 세대를 확인하는 기존 작업 직렬화 장치를 사용한다. 사전 준비가 끝나면 같은 출력 형식·샘플 수의 상주 경로는 파이프라인을 다시 만들지 않는다. 출력 형식이나 샘플 수가 바뀌면 새 파이프라인이 필요할 수 있다. 비상주 스트리밍의 행렬·타일 전용 파이프라인은 범위에 포함되지 않으며 스트리밍 준비 시 생성한다. `warm_up()`은 `first_frame_ready()`와 같은 작업을 하는 호환 메서드다.

초기화 시간은 호스트에서 `performance.now()` 같은 단조 시계로 측정한다. 별도 WASM 함수나 커널 내부 타임스탬프는 필요하지 않다. `FiggyChart.create_with_progress`의 시작·완료 시각과 진행 콜백 수신 시각을 기록하면 된다. 일반 생성은 창·렌더러 준비, 각 비동기 렌더링 진입점, `web.create / chart resources`, `web.create / first frame` 순서로 started/finished 알림을 보낸다. 파이프라인별 첫 제출을 비교하려면 표준 등록·설정·프레임 API로 선·점·오차 막대 작업을 각각 구성하고 같은 외부 시계로 측정한다.

피킹 사용 여부를 비교할 때는 비활성 조건에서 `prewarm_gpu_picking()`을 호출하지 않는다. 활성 조건에서는 `web.create / first frame / finished` 콜백 이후 이 메서드를 직접 호출하고 소요 시간을 따로 기록한다.

`FiggyChart.create`는 피킹을 켜지 않는다. 첫 선택 전에 준비하려면 `await chart.prewarm_gpu_picking()`을 호출한다. 이 메서드와 `pick_point` / `pick_data`는 렌더러의 `enable_gpu_picking_async`를 거친 뒤 현재 차트의 피킹 캐시를 준비한다. 파이프라인·캐시·리비전은 렌더러가 관리하므로 반복 호출과 재시도도 저장된 활성화 오류를 포함한 같은 상태를 사용한다.

`Renderer`는 차트별 설정과 시리즈 순서, `ColumnPool`, 피킹 파이프라인, 현재 차트용 피킹 캐시 하나, 대기 중인 정리 작업을 관리한다.

네이티브·임베드 호스트는 `enable_gpu_picking()`, 필요에 따른 `prepare_gpu_picking_for_chart(chart_id)`, `pick_chart(chart_id, GpuPickRequest)` 순서로 사용한다. `WindowedRenderer::pick_chart_at`은 현재 surface에 맞는 패널 위치와 배율도 계산한다. 호스트가 축 변환이나 데이터 영역 자르기 계산을 중복할 필요는 없다.

점·막대·셀·등고선을 구분하는 선택에는 `pick_chart_data` / `WindowedRenderer::pick_chart_data_at`을 사용한다. 결과에는 종류별 식별자와 `distance_px`가 들어간다. 막대·행렬 선택용 컴퓨트 셰이더는 그리기와 같은 좌표 변환·풀·스타일·격자·레벨 표·도형 계산 함수를 사용한다. CPU는 끝점의 f64 좌표, 막대 사각형, 셀 경계, 등고선 선분을 복원해 보관하지 않는다. 결과를 `set_picked_data`로 전달하면 그리기에 사용한 같은 바인드 그룹으로 선택 표시를 그리므로 축이나 데이터가 바뀌어도 위치가 맞는다.

<a id="32-렌더-루프--requestanimationframe--renderer-stamp"></a>

### 3.2 렌더링 루프와 프레임 갱신 판단

`<figgy-chart>`는 `requestAnimationFrame` 콜백에서 데스크톱 예제와 같은 갱신 판단을 수행한다. 저수준 커널을 직접 사용한다면 이 루프도 호스트가 구현해야 한다. 다음은 실제 `frame()`의 상태 변화를 요약한 의사 코드다.

```rust
renderer.sync_external_invalidations()?; // process-global font registration
let stamp = renderer.chart_render_stamp(chart_id)?;
let renderer_dirty = stamp.needs_draw_since(last_presented_stamp.as_ref());
let raster_dirty = stamp.needs_raster_since(last_presented_stamp.as_ref());
let annotation_dirty = stamp.needs_annotation_raster_since(last_presented_stamp.as_ref())
    || annotation_selection_dirty;

match frame_decision(
    renderer_dirty,
    raster_dirty,
    view_dirty,
    redraw_pending,
    renderer.has_pending_maintenance(),
) {
    Clean => return Ok(()),
    MaintenanceOnly => {
        renderer.process_pending_maintenance()?; // surface acquire/draw 없음
        return Ok(());
    }
    Draw { refresh_raster } => {
        renderer.process_pending_maintenance()?;
        let stamp = renderer.chart_render_stamp(chart_id)?;
        if refresh_raster {
            // 축 이미지와 함께 주석 이미지도 다시 그린다.
            renderer.refresh_axis_with_selection(
                &mut view,
                &display_chart,
                rect,
                selected_id.as_deref(), // 선택된 요소 id, 예: Some("data_area")
            )?;
        } else if annotation_dirty {
            renderer.refresh_annotations(&mut view, &display_chart, rect, selected_annotation)?;
        }
        renderer.draw(clear, &items)?;

        // submit/present 성공 뒤에만 onscreen 상태를 전진시킨다.
        view_dirty = false;
        redraw_pending = false;
        annotation_selection_dirty = false;
        last_presented_stamp = Some(stamp);
    }
}
```

`WindowedRenderer::draw`는 `Renderer::prepare`와 `Renderer::paint_prepared`를 한 번에 실행한다. 준비 단계는 `&mut self`로 파이프라인 준비, 좌표 변환 유니폼 기록, 경로 길이 컴퓨트 실행을 수행한다. 기록 단계는 `&self`로 그리기 명령만 만든다. WASM 래퍼처럼 렌더러를 단독으로 관리하는 호스트는 이 통합 호출을 사용하고, 공유 참조만 받는 paint 콜백에서는 두 단계를 나눠 호출한다. 자세한 내용은 [USAGE.md의 egui 통합 패턴](USAGE.md#egui-통합-패턴-요약)을 참고한다.

변경이 없는 rAF에서도 다음 콜백 예약, DPR 비교와 WASM 상태 확인은 수행한다. GPU 컬럼 준비, surface 획득, 그리기·제출·화면 표시는 생략한다. 갱신이나 그리기가 실패하면 마지막 표시 기록과 호스트 플래그를 유지해 다음 rAF에서 다시 시도한다. 이 최적화는 이전 화면이 그대로 유효할 때만 적용하며 데이터 샘플링·LOD·데시메이션이나 시간에 따른 프레임 생략은 하지 않는다.

`interrupt_render()`가 스트림 취소를 예약하면 다음 프레임도 예약된다. 프레임 시작의 `process_pending_maintenance()`는 취소를 먼저 처리하고, 그 뒤에 스트림 화면과 상주 차트 중 어느 경로로 그릴지 결정한다. 풀 재배치가 없어도 취소는 처리된다. 같은 커널에 다른 차트의 설정과 시리즈를 복원하면 복귀 첫 프레임부터 창 크기에 맞는 좌표와 잘라내기 영역을 사용하며, 저장된 축 범위는 유지한다. 객체 선택이나 축 조작으로 다시 그릴 필요가 없다.

`crates/web/tests/stream-resident-return-probe.html`은 **상주 차트 확대 → 스트림 → 상주 차트 복귀**의 첫 화면을 실제 캔버스 픽셀로 비교한다. 기본 창은 1200×800이며 `?width=600&height=900&partial=1`로 세로 창에서 스트림 도중 복귀를 검사할 수 있다. `cancelOnly=1`을 추가하면 설정 변경 없이 취소만 해도 다음 프레임에 반영되는지 함께 검사한다.

컬럼 교체·제거·재배치는 임시 풀, 영향을 받는 차트 상태·리비전, 새 피킹 캐시를 먼저 준비한다. 동기 준비 중 오류가 나면 기존 상태를 유지한다. 성공하면 풀·차트·피킹·정리 상태를 추가 할당 없이 반영한다. Rust `Renderer::remove_column`은 해당 컬럼을 참조하는 시리즈만 함께 제거하고 범례 문서는 바꾸지 않는다. 범례 설정도 함께 바꿔야 한다면 `Renderer::remove_column_with_chart_config`로 한 번에 반영한다. 웹 `FiggyChart.remove_column`은 자동 범례의 해당 행도 지운다([데이터 등록·제거와 메모리 관리](#등록해제-모델--메모리는-내부-자동-관리) 참고).

`FiggyChart::load_demo()`도 데모 상태 전체를 한 번에 교체한다. 컬럼 4개, 최종 설정과 시리즈 순서, 피킹 상태, 웹의 컬럼 리비전·스타일·라벨·색상 메타데이터, 유효한 범위 캐시를 함께 반영한다. 확정 전에 동기 오류가 나면 모두 이전 상태를 유지한다. 트랜잭션 중에는 범위 계산 명령을 제출하지 않으며, 변경된 컬럼을 참조하던 캐시는 제거한 뒤 필요할 때 다시 만든다. 이 작업에는 풀 용량과 같은 임시 GPU 버퍼 하나와 스테이징 버퍼 4개가 추가로 필요하다. 재배치 백업이 남아 있으면 원본 풀·백업·임시 풀이 잠시 공존한다.

<a id="33-데이터-입력--명시적-registerupdate와-f32-물리-lane"></a>

### 3.3 데이터 등록·교체와 GPU 저장 형식

GPU 풀은 논리값 하나를 **항상 f32 두 개**로 저장한다. 일반 컬럼은 `(value as f32, 0)`, `Float64Array` / `HiLoColumnSource`는 `(hi: f32, lo: f32)`를 기록한다. 셰이더의 f64 연산 대신 두 f32의 합을 이용해 큰 절대값 안의 작은 차이를 보존한다. 이 저장 형식은 네이티브와 WASM에서 같다. 다음 예는 매핑된 스테이징 버퍼를 준비한 뒤 실행하는 내부 업로드 코드다.

```rust
// 일반 컬럼과 hi/lo 컬럼이 같은 호출을 사용한다.
let mut view = staging.slice(..).get_mapped_range_mut();
let writer = ColumnPairWriter::new(view.slice(..)); // renderer pool 내부, crate-private
// ColumnSource는 `(value as f32, 0)`, HiLoColumnSource는 `(hi, lo)` 쌍을 기록한다.
let stats = source.write_f32_pair_le_into_with_stats(writer);
drop(view);
staging.unmap();
enc.copy_buffer_to_buffer(&staging, 0, &pool, offset, None);  // 이후는 GPU 내부 복사
```

네이티브에서는 원본 참조를 빌려 변환 결과를 매핑된 업로드 버퍼에 직접 쓴다. 데이터 소스는 값을 기록하는 같은 반복문에서 `ColumnUploadStats`도 계산하므로 렌더러가 쓰기 전용 영역을 다시 읽지 않는다. 최소 양수 값은 일반 컬럼의 실제 `value as f32`, hi/lo 컬럼의 `hi as f64 + lo as f64`를 기준으로 하며 유한한 양수만 포함한다.

사용자 정의 `ColumnSource` / `HiLoColumnSource`도 값 기록과 통계 계산을 함께 하는 메서드를 구현해야 한다. 구현이 빠지면 컴파일 오류가 난다. 매핑된 바이트를 다시 읽거나 부정확한 대체 경로로 처리하지 않는다.

WASM에서는 다음과 같은 메모리 경계 복사가 추가된다.

1. JavaScript에서 받은 데이터는 JS 힙에서 WASM 선형 메모리로 한 번 복사한다. WASM 안에서 생성하거나 직접 가져온 데이터에는 이 복사가 없다.
2. wgpu 30.0.1의 웹 백엔드는 쓰기 뷰를 만들 때 브라우저의 매핑 영역을 임시 WASM `Vec`로 복사하고, 뷰를 해제할 때 변경 내용을 되돌려 복사한다. 이는 wgpu가 JS·WASM 경계를 처리하는 방식이며 GPU 결과 읽기와는 다르다. 네이티브 업로드에는 이 복사가 없다.

입력 배열은 데이터 정밀도에 맞춰 고른다.

- **`Float32Array`**: 일반 좌표에 권장한다. JS·WASM 경계에서 원소당 4바이트를 전달하며, 빌린 데이터에서 `(value, 0)` 기록과 통계 계산을 한 번에 수행한다.
- **`Float64Array`**: 큰 절대 좌표에 사용한다. 경계 전달량과 GPU 저장량은 원소당 8바이트다. 최소·최대 메타데이터뿐 아니라 GPU 좌표 계산도 hi/lo 쌍을 사용하므로 타임스탬프 같은 큰 값에서 f32 하나로 표현할 수 없는 작은 차이를 보존한다.

`crates/web`에는 WASM 메모리에 버퍼를 할당해 주는 메서드가 없다. 인자 변환 비용까지 줄이려면 버퍼를 할당해 포인터와 길이를 돌려주는 WASM 함수를 직접 추가하고, JS가 `new Float32Array(memory.buffer, ptr, len).set(src)`로 채우게 한다. 이 방식도 경계 복사는 한 번 필요하며 wasm-bindgen의 인자 변환 과정만 줄인다.

공개 API는 등록과 교체를 구분한다:

```js
chart.register_column_f32("x", xs);          // 새 id만; 기존 id면 오류
chart.update_register_column_f32("x", next); // 기존 id만; 없으면 오류

chart.register_column_f64("time", times);          // Float64Array → hi/lo
chart.update_register_column_f64("time", nextTimes);
```

행렬 데이터는 컬럼 수가 많을 수 있으므로 연속된 배열 하나로 묶어 등록할 수 있다.

```js
// z는 컬럼 ids.length개 × 컬럼당 값 valuesPerColumn개를 id 순서로 이어 붙인 배열이다.
const valuesPerColumn = 2000;
const ids = Array.from({ length: 400 }, (_, c) => `z${c}`);
chart.register_columns_f32(ids, z, valuesPerColumn);   // 업로드 1회
// Float64Array라면 위 호출 대신 사용한다. f64는 hi/lo 쌍을 유지한다.
// chart.register_columns_f64(ids, z64, valuesPerColumn);
```

GPU 풀은 값마다 8바이트를 쓴다. 위 배치는 값 80만 개, 약 6.4MB로 처음 만든 16MiB 웹 풀에 들어간다. 더 큰 배치는 자동 확장이 켜진 기본 상태에서 풀이 늘어나 그대로 들어간다. 풀 크기의 상한은 장치의 버퍼 크기 한도와 스토리지 바인딩 한도 중 작은 값이다. 렌더러는 어댑터가 지원하는 한도를 그대로 요청하므로 이 상한은 하드웨어의 한도다. 이를 넘으면 `device_limit` 오류가 난다. `set_pool_auto_growth(false)`로 확장을 끄면 풀 공간이 부족할 때 `pool_space` 오류가 난다.

일괄 등록은 컬럼별 배열 목록 대신 연속된 배열 하나를 받는다. 직사각형 행렬의 메모리 배치를 그대로 사용하면 JS에서 컬럼마다 순회하고 WASM 경계를 반복해서 넘는 비용을 줄일 수 있다. 업로드할 때 각 컬럼은 전달된 버퍼의 일부를 빌려 스테이징 버퍼에 기록하므로 컬럼별 복사본을 만들지 않는다. JS·WASM 경계 복사는 앞서 설명한 규칙을 따른다.

- 새 ID만 등록할 수 있다. 배치 내 중복 ID, 이미 등록된 ID, `data.length !== ids.length × valuesPerColumn`이면 전체를 거부한다. 실패한 배치는 아무 ID도 등록하거나 업로드하지 않는다.
- 컬럼 길이가 서로 다른 데이터는 개별 `register_column_*` 호출로 등록한다.
- 리비전은 배치 단위가 아니라 컬럼마다 증가한다.
- `register_columns_f64`도 `(hi, lo)` 정밀도를 유지하며 f32 하나로 축소하지 않는다.

박스 플롯용 레코드와 가변 길이 그룹(ragged) 컬럼은 각각 한 번의 배치 업로드로 등록한다.

```js
// 필드 이름을 키로, 같은 길이의 Float32Array/Float64Array를 값으로 갖는 객체. 행 하나가 박스 하나다.
chart.register_record("boxes", { median, q1, q3, whisker_low, whisker_high, mean });
// 그룹 g는 values[offsets[g]..offsets[g + 1]]이다. offsets는 0에서 시작하고 줄어들지 않으며 values.length로 끝난다.
chart.register_ragged("outliers", values, offsets);
```

- `register_record`의 필드는 `median`, `q1`, `q3`, `whisker_low`, `whisker_high`(필수)와 `mean`, `sample_count`, `ci_low`, `ci_high`(선택)이다. 없는 선택 필드는 NaN으로 읽고, 모든 필드가 NaN인 행은 빠진 박스다. 알 수 없는 필드, 빠진 필수 필드, 길이가 다른 필드는 오류다. 배열은 복사하지 않고 스테이징 버퍼에 바로 기록한다.
- `register_ragged`는 `Float64Array` 두 개를 `(hi, lo)` 쌍으로 업로드한다. offsets가 줄어들거나 0에서 시작하지 않거나 정수가 아니거나 values.length로 끝나지 않으면 업로드를 버리고 오류를 낸다.
- 두 함수 모두 같은 ID로 다시 호출하면 교체하며, 실패하면 기존 상태를 유지한다. 컬럼, 레코드, ragged 컬럼은 ID 공간 하나를 공유하므로 이미 컬럼인 ID는 거부한다. 레코드와 ragged 컬럼은 상주 데이터만 지원하며 스트리밍 ID는 거부한다.
- `RangeBars` 시리즈는 `x_column`(위치), `y_column`(끝값), `start`와 선택 컬럼(`whiskers`의 `low`·`high`, `color_by`의 합계 표시 또는 상태 컬럼, `progress`)을 일반 컬럼으로 읽는다. 모든 컬럼은 위치 컬럼과 길이가 같아야 한다. [SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md#rangebars-render_type)를 참고한다.
- `BoxPlot` 시리즈는 `y_column`으로 레코드를, `outliers`로 ragged 컬럼을, `x_column`으로 범주 위치 컬럼(`0..n-1`)을 가리킨다. [박스플롯 문서](#box-plot-documents)를 참고한다.
- 박스플롯의 `violin`은 반쪽마다 곡선 ragged 컬럼과 밀도 컬럼을, `points`는 원시 점의 범주 위치 컬럼과 값 컬럼을 가리킨다. 밀도는 호스트가 계산해 올리고, 점의 흔들림(jitter)은 셰이더가 점 번호로 계산하므로 별도 컬럼이 없다. 상자 없이 바이올린만 그리려면 `y_column`을 비운다.

빈 배열은 거부한다. 유효한 `update_register_*` 호출은 같은 값이라도 매번 교체 업로드를 수행하며, 실패하면 기존 상태를 유지한다. 해시만 비교해 업로드를 생략하지 않는다. `set_series`는 어떤 등록 컬럼을 그릴지만 바꾸고 컬럼을 업로드하지 않는다.

<a id="34-이벤트-입력--포인터를-모델-정책으로-그대로-전달"></a>

### 3.4 포인터 이벤트 전달

선택·드래그·크기 조절 정책은 `model`의 `Selectable` / `Draggable` / `Resizable` / `HitMap`에 정의돼 있으며 WASM에서도 같은 코드를 사용한다. 웹 래퍼가 포인터 좌표를 변환하므로 일반 호스트는 별도 처리가 필요 없다. 저수준 커널을 직접 사용할 때는 캔버스 이벤트의 위치를 픽셀 좌표로 바꿔 전달한다.

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

웹 래퍼는 이벤트마다 캔버스의 CSS 크기와 실제 픽셀 크기의 비율로 포인터 좌표를 계산한다. `FiggyChart`는 문서 영역 `Config::document_area()`의 가로세로 비율을 유지해 현재 surface에 맞추고 남는 공간에 여백을 둔다. 문서 영역은 `chart_area`와 모든 서브플롯 `chart_area`의 합집합이며, 서브플롯이 없으면 `chart_area`와 같다. 히트 테스트와 주석 조작은 캔버스 픽셀에서 이 배치대로 판정하고, 요소 드래그의 시작점과 드래그·크기 조절의 이동량은 같은 배치와 배율을 거꾸로 적용해 문서 좌표로 환산한다. 브라우저 창 크기를 바꾸는 것은 미리보기 배율만 바꾸며 출력 문서나 폰트 크기는 바꾸지 않는다.

`on_press`의 반환값은 이번 누름이 무언가를 잡았는지를 나타낸다. 요소, 선택된 요소의 크기 조절점, 주석 항목이나 그 핸들을 누르면 `true`, 빈 곳을 누르면 `false`다. 현재 선택 상태는 `has_selection()`으로 확인한다. 이 메서드는 요소나 주석이 선택돼 있으면 `true`다. 주석 층이 포인터를 받지 않을 때(`"Annotations"`가 아닌 레이어이거나 주석 층이 숨겨졌을 때) 빈 곳을 누르면 요소 선택은 지워지지만 `select_annotation`으로 고른 주석은 그대로 남으므로, `on_press`가 `false`를 돌려준 뒤에도 `has_selection()`은 `true`일 수 있다. `<figgy-chart>`가 보내는 `figgy-select`의 `selected`는 `on_press`의 결과이고, `figgy-release`의 `selected`는 `has_selection()`의 결과다.

<a id="35-이벤트-출력--customevent로-프레임워크-중립"></a>

### 3.5 CustomEvent로 결과 알림

선택 변경이나 드래그 종료는 웹 래퍼가 `CustomEvent`로 알린다. React·Vue·Svelte에서도 표준 이벤트 구독 방식을 사용하면 된다. 이벤트는 사용자 정의 요소에서 `bubbles: true`, `composed: true`로 발생한다.

```js
chartEl.addEventListener("figgy-select", (e) => {
  console.log(e.detail.selected);
});
chartEl.addEventListener("figgy-init-progress", (e) => {
  // { scope, stage, phase: "started" | "finished" } — ready 이전에도 발생
  console.log(e.detail.scope, e.detail.stage, e.detail.phase);
});
```

<a id="36-png-export--async-필수-uint8array-반환"></a>

### 3.6 PNG를 `Uint8Array`로 비동기 내보내기

```rust
pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
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
        .export_document_png_bytes_with_clear_async(
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

`export_png`는 문서 출력이다. `overlays`가 있으면 기본 레이어와 오버레이 레이어를 캔버스와
같은 순서로 한 이미지에 합성하고, 없으면 패널 출력(`export_panel_png_bytes_with_clear_async`)과
바이트 단위로 같은 결과를 낸다. `subplots`가 있으면 이미지는 모든 차트 영역의 합집합 크기이고,
캔버스도 그 합집합을 표면에 맞춰 그린다.

<a id="raw-kernel-call-ownership"></a>

#### 저수준 커널 호출 소유권

`export_png`는 Promise가 끝날 때까지 저수준 `FiggyChart`를 `&mut self`로 빌린 상태를 유지한다. wasm-bindgen은 겹친 호출을 대기열에 넣거나 직렬화하지 않는다. 그동안 같은 커널을 호출하면 동기 getter, `frame()`, 편집, 크기 변경, 피킹, `free()` 모두 `"recursive use of an object"` 예외를 던질 수 있다. 같은 객체에 대한 작업을 `Promise.all`로 묶지 않는다. 커널 인스턴스가 여러 개라면 인스턴스마다 작업 게이트를 따로 둔다.

저수준 커널을 직접 쓰는 호스트에서는 모든 접근 경로가 인스턴스별 게이트 하나를 확인해야 한다. 작업 중에는 렌더링과 포인터 호출을 멈추거나 건너뛰고, 앱의 작업 요청은 명시적으로 거부하거나 뒤로 미룬다. 게이트는 `await` 전체에 걸쳐 유지하고, 작업이 성공하든 거부되든 `finally`에서 해제한다. 작업이 빌리고 있는 커널은 해제하지 않는다. 이미 생성됐고 수명이 고정된 커널에서는 다음과 같이 최소한으로 구현할 수 있다.

```js
let busy = false;

async function withKernelOperation(operation) {
  if (busy) throw new Error("chart is busy");
  busy = true;
  try {
    return await operation(kernel);
  } finally {
    busy = false;
  }
}

function tick() {
  requestAnimationFrame(tick);
  if (!busy) kernel.frame();
}

const png = await withKernelOperation((chart) => chart.export_png(2.0));
// getter·편집·포인터 처리·크기 변경·해제도 `busy`를 확인해야 한다.
```

`<figgy-chart>` 래퍼는 비동기 커널 작업을 위해 연결 세대를 확인하는 게이트를 이미 갖고 있다. 경쟁하는 앱 호출은 `"figgy chart is busy"`로 거부하며, 일반적인 호출 대기열이 아니다. 래퍼를 쓸 때는 래퍼에 있는 메서드를 저수준 커널로 우회해 호출하지 않는다. 래퍼에 없는 메서드는 [`element.kernel`](#figgy-chart-kernel)로 호출한다. 연결 해제와 재연결을 지원하는 호스트는 [3.9절](#39-async-메서드와-객체-잠금-필독)의 수명 규칙도 따라야 한다.

동기 출력 함수 `export_panel_png_bytes`는 웹 빌드에서 제외된다. 웹에서는 비동기 버전을 사용해야 한다.

#### SVG와 PDF 내보내기

`FiggyChart::export_svg(scale)`는 SVG 문서를 `String`으로, `export_pdf(scale)`는 PDF를 `Uint8Array`로 비동기 반환한다. 둘 다 `export_png`와 같은 문서를 차트의 배경색 위에 그린다. 장식 요소와 주석은 벡터로, 글자는 글리프 윤곽선으로 기록하고, 패널 묶음의 데이터는 PNG 출력과 같은 픽셀의 이미지로 넣는다(자세한 내용은 [USAGE.md](USAGE.md)의 SVG와 PDF 내보내기). 빌림 규칙과 작업 게이트는 `export_png`와 같다.

```js
const svg = await chart.export_svg(1.0);   // String
const pdf = await chart.export_pdf(1.0);   // Uint8Array
const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
```

`<figgy-chart>` 래퍼의 `export_svg(scale)` / `export_pdf(scale)`도 `export_png`처럼 `"export"` 작업 게이트를 거친다. 스트리밍으로 표시 중인 차트에서는 커널을 호출하기 전에 거부한다. 스트리밍은 SVG·PDF 출력으로 확장하지 않는다.

<a id="37-프리셋--fieldless-enum-그대로-노출"></a>

### 3.7 축·색상 프리셋

`model::AxisPreset`의 축 모양 5종과 `model::ColorCycle`의 색상 순서 5종은 추가 필드가 없는 열거형이다. wasm-bindgen이 이를 정수 열거형으로 내보내며, 웹 래퍼는 같은 이름의 열거형과 `From` 변환을 제공한다.

```js
chart.apply_axis_preset(AxisPreset.OpenOutward);   // 4축 일괄
chart.apply_color_cycle(ColorCycle.ColorblindSafe); // 시리즈 재색칠 + 범례 동기
color_cycle_css(ColorCycle.Vivid);  // → ["rgb(0 32 240 / 1)", …] 호스트 스와치용
```

<a id="38-ssot-io--configseries-전체를-json으로-라운드트립"></a>

### 3.8 설정과 시리즈를 JSON으로 읽고 쓰기

설정의 일부만 바꾸려면 렌더러가 단일 원본(SSOT)으로 보관하는 설정 문서를 직접 편집한다. 현재 설정을 `get_config`로 읽고 바꿀 필드만 고친 뒤 전체 설정을 `set_config`로 다시 넣는다. 다음 예는 두 X축의 범위만 바꾸고 눈금 간격·보조 눈금 수·라벨 형식 같은 나머지 설정은 그대로 둔다.

```js
const rangeConfig = JSON.parse(chart.get_config());
for (const axis of [rangeConfig.bottom_x, rangeConfig.top_x]) {
  axis.min = 20;
  axis.max = 80;
}
chart.set_config(JSON.stringify(rangeConfig));
// 다음 프레임부터 새 범위로 그린다. 호스트 루프에서 프레임을 요청한다.
```

범위와 그에 맞는 눈금·라벨 설정을 함께 계산하려면 `auto_fit_x`, `auto_fit_y`, `auto_fit_all`, `auto_fit_colorbar`를 사용한다. `auto_fit_all`은 활성 레이어의 축만 맞춘다(5절 `<figgy-chart>` 공개 API 표의 「범위 맞춤」 참고). `set_title`은 필요하면 제목 영역도 넓힌다. 기존 여백을 그대로 두려면 `chart_title.text`를 직접 편집한다. `set_x_title`과 `set_y_title`은 축 제목 텍스트만 바꾼다. 네이티브에서 같은 작업은 [direct SSOT edits and convenience helpers](API.md#direct-ssot-edits-and-convenience-helpers)를 참고한다.

직접 넣은 설정도 일반 검증과 렌더링 안전 한도를 거친다. 눈금 설정을 그대로 두면 데이터 좌표에서의 의미는 유지되지만, 화면상 눈금 위치나 표시되는 라벨 수는 바뀔 수 있다. `set_config`는 교체되는 설정에 묶인 대기 중인 자동 맞춤 작업도 취소한다. 다시 맞춰야 할 때만 새로 요청한다.

`Config`와 `Vec<SeriesConfig>`는 GPU 핸들이 없는 데이터 구조다. `model`의 `serde` 기능을 켜면 전체를 JSON으로 직렬화할 수 있다. 이 기능은 기본적으로 꺼져 있어 불필요한 의존성을 추가하지 않는다. 웹 래퍼의 `get_config` / `set_config` / `get_series` / `set_series`로 설정을 읽고 수정할 수 있다.

```js
// 먼저 auto-fit으로 범위를 정한 뒤, 설정을 읽어 원하는 필드를 고쳐 다시 넣는다.
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

`set_series(json)`는 `Vector` 시리즈를 두 형식으로 받는다. `components.Points`는 기준점과 인덱스가 맞는 `u` / `v` 컬럼을, `components.Grid`는 좌표 컬럼 위의 행렬 두 개를 사용한다. 색상 매핑을 쓰는 벡터 시리즈에는 `colorbar`가 필요하다. 선택한 화살표나 셀의 `selection_details`는 `u`, `v`를 돌려주고, 색상 매핑을 쓰면 `color_value`도 돌려준다. 자세한 형식은 [SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)를 참고한다.

<!-- contour-contract: scope=wasm max-levels=1024 -->
`set_series(json)`에서 `Contour`와 `HeatmapContour`의 `contour.levels`는 `0..=1024`개를 허용한다. 1025개 이상이면 JavaScript 예외를 반환하고 이전 설정·시리즈·GPU 스타일을 유지한다. 다음 프레임도 이전 상태로 그린다. WASM에만 더 작은 상한을 두거나 배열을 잘라서 처리하지 않는다.
등고선 라벨도 자동·직접 지정 모두 1024개까지 지원한다. `spacing_px`는 라벨을 숨겼거나 앵커를 직접 지정했더라도 유한한 양수여야 한다. 자동 배치에서는 허용 범위로 제한한 프레임·출력 배율과 간격의 곱도 검사한다. 이 곱이 넘치거나 라벨 아틀라스가 어댑터의 텍스처 크기 한도를 넘으면 GPU 상태를 바꾸기 전에 실패해 이전 차트와 자원을 유지한다.

`set_config`는 JSON을 검증한 뒤 `set_chart_config(chart_id, config)`로 렌더러의 설정을 교체한다. 설정이 바뀌면 차트의 렌더 스탬프도 바뀌므로 다음 `frame()`이 데이터 화면과 축 이미지를 다시 그린다([3.2절](#32-렌더-루프--requestanimationframe--renderer-stamp) 참고).
축 스케일을 바꾸면 `major_spacing`의 단위도 바뀐다. 선형축에서는 데이터 단위이고 로그축에서는 10배 간격을 뜻하는 decade 단위다. 웹의 `auto_fit_x` / `auto_fit_y`와 Rust의 `Chart::set_x_range` 같은 범위 헬퍼는 이를 맞춰 주지만(웹 API에는 `set_x_range`가 없다) JSON을 직접 편집할 때는 함께 수정해야 한다. `AxisOptions.inverted`도 JSON 필드로 지정한다. 축·데이터·피킹이 같은 방향 반전을 적용한다.

전체 필드와 열거형 값, serde 표현 규칙, 함께 수정해야 하는 옵션은 [SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)를 참고한다. SCHEMA.md의 예시 JSON은 소스와 자동으로 대조하지 않으므로, 정확한 형식은 `get_config()` / `get_series()`의 실제 출력으로 확인한다.

<a id="39-async-메서드와-객체-잠금-필독"></a>

### 3.9 비동기 호출 중 객체 접근 제한

`&mut self`를 빌리는 비동기 메서드가 실행 중일 때 같은 객체에 다시 접근하면 `"recursive use of an object"` 예외가 발생할 수 있다. wasm-bindgen은 호출을 대기열에 넣지 않는다. `<figgy-chart>` 래퍼는 작업 중인 객체에 대한 추가 호출을 거부한다. 저수준 커널을 직접 사용한다면 다음 규칙을 지켜야 한다.

- rAF 루프는 WASM을 호출하기 전에 다음 `requestAnimationFrame(tick)`을 예약한다. 예외가 발생해도 루프가 끊어지지 않게 하기 위해서다.
- 생성·연결, 두 전체 사전 준비 메서드, `prewarm_gpu_picking`, 출력(`export_png`, 스트림 출력의 `finish_stream_export` / `cancel_stream_operation_and_wait`), `first_frame_ready` / `warm_up`, `ensure_extent_engine`, 상주 `auto_fit_all`, `pick_point` / `pick_data`, `release_unused_gpu_memory`, `cancel_streaming_and_wait`가 실행되는 동안 연결 세대와 커널을 확인하는 `busy` 토큰을 유지한다. 저수준 `selection_details()`의 비동기 GPU 읽기 중에도 같은 접근 제한을 적용한다. 그동안 `frame()`·포인터·크기 변경·대리 호출은 생략하거나 거부한다. 상주 `auto_fit_all`은 완료 시 설정을 직접 바꾸므로 실행 중 `frame()`을 호출할 수 없다.
- 웹 래퍼의 스트리밍 `auto_fit_all()`은 위 저수준 비동기 메서드를 호출하지 않는다. 렌더러에 범위 계산을 요청하고 스트리밍 작업의 `done`을 기다린다. 청크 공급과 커널 호출을 계속해야 하므로 일반 `busy` 토큰을 잡지 않으며, 작업 중 `busy`가 `false`일 수 있다. 스트리밍 실행기가 변경 요청을 최신 상태에 맞춰 반영한다.
- 작업 중 들어온 크기 변경과 포인터 해제는 마지막 것만 보관했다가, 작업이 끝나면 WASM에 적용한 뒤 토큰을 해제한다. 연결이 끊겨도 작업이 사용 중인 커널은 작업이 끝난 뒤 해제한다. 이전 연결 세대의 작업이 끝났다고 새 세대의 토큰을 해제하지 않는다.

웹 래퍼 사용 예제는 `crates/web/index.html`에 있다. 저수준 커널은 위 규칙을 직접 구현해야 하는 경우에 사용한다.

## 4. 제약과 주의사항

- **메인 스레드 사용**: 축·장식의 CPU 렌더링은 메인 스레드에서 실행한다. 이 래스터는 축·라벨·장식이나 캔버스 크기가 바뀐 프레임에서만 다시 실행하며, 비용은 캔버스 크기·글자 수·환경에 따라 달라진다. `FiggyChart.create`와 `FiggyBoundedChart.create`는 `HtmlCanvasElement`만 받으므로 이 패키지는 Web Worker에서 실행할 수 없다. Worker에서 그리려면 `OffscreenCanvas`를 `wgpu::SurfaceTarget`으로 넘기는 래퍼를 Rust로 직접 작성한다. SharedArrayBuffer를 쓰는 WASM 스레드를 사용하려면 서버의 COOP/COEP 헤더로 교차 출처 격리를 설정해야 한다.
- **WebGPU 지원**: HTTPS나 루프백 주소에서 `navigator.gpu` 존재 여부와 어댑터·장치 요청 성공을 확인한다. 지원 여부는 브라우저 버전뿐 아니라 OS·GPU·드라이버·브라우저 정책에 따라 다르다. 이 렌더러는 컴퓨트·스토리지 버퍼와 WebGPU 초기화를 사용하므로 wgpu의 `webgl` 기능만 켜서 WebGL2로 대체할 수 없다. 일반 환경에서는 브라우저의 기본 GPU 선택을 따른다. 실제 GPU가 없는 Linux 검증 환경에서만 별도 실행 설정으로 SwiftShader Vulkan을 사용할 수 있다.
- **폰트**: 내장 Liberation Sans에는 한중일 문자가 없다. 한글을 표시하려면 `register_font(Uint8Array)`로 TTF/OTF/TTC 파일을 등록한다. 반환된 글꼴 이름을 `font`에 지정하면 그 폰트로 그린다. 지정하지 않아도 선택한 폰트에 없는 한글은 글자마다 등록 폰트에서 찾아 그린다. fontdb는 WOFF2를 읽지 못한다. 스케치 모드는 내장 Comic Neue(OFL)를 사용한다. 이 폰트에 없는 한중일·그리스 문자 등은 글자마다 `font`의 폰트로 그리고, 그 폰트에도 없으면 등록 폰트와 Liberation Sans에서 찾는다. 따라서 등록한 한글 폰트는 스케치 모드에서도 사용할 수 있다.
- **동기 대기 금지**: WASM에서 `pollster::block_on`을 사용하면 교착 상태가 발생할 수 있다. 구현은 비동기로 작성하고 동기 래퍼는 `#[cfg(not(target_arch = "wasm32"))]`로 네이티브에서만 빌드한다.

## 5. 빌드 산출물(`crates/web` → `pkg/`)

브라우저 패키지의 소스는 `crates/web`에 있으며 패키지명은 `figgy`다. 다음 명령으로 release 빌드를 만든다.

```bash
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
```

현재 `wasm-opt`는 꺼져 있다. wasm-pack에 포함된 Binaryen이 최신 rustc 출력의 일부 기능에서 비정상 종료되기 때문이다(`crates/web/Cargo.toml` 참고). Rust의 release 최적화는 적용한다. 추가 용량 절감이 필요하면 호환되는 최신 Binaryen으로 검증한 뒤 다시 활성화할 수 있다.

생성 파일은 `crates/web/pkg/`에 저장하며 프론트엔드에 함께 배포한다.

| 파일 | 내용 |
|---|---|
| `figgy_bg.wasm` | WASM 본체. wgpu, Liberation Sans 4종, Comic Neue 2종, 수식용 New Computer Modern Math 1종을 포함한다. 크기는 툴체인·기능·최적화에 따라 달라진다. |
| `figgy.js` | WASM과 JavaScript를 연결하는 ES 모듈 — `import init, { FiggyChart, … }` |
| `figgy.d.ts` | 자동 생성한 TypeScript 타입 정의. 저수준 WASM API 형식을 확인할 수 있다. |
| `package.json` | npm용 패키지 메타데이터 |

일반 웹 API는 `crates/web/figgy-chart.js`가 제공한다. `pkg/`에서 생성되는 파일이 아니므로 함께 배포해야 한다. 이 파일은 내부 캔버스, rAF, DPR·포인터 좌표 변환, 선택·드래그·크기 조절, ResizeObserver, 준비 이벤트와 비동기 호출 직렬화를 담당한다. 사용 예제는 `crates/web/index.html`, 전체 JSON 설정은 [SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)를 참고한다.

로컬에서는 HTTP 서버로 열어 확인한다.

```bash
cd crates/web && python3 -m http.server 8137   # wasm은 file://로 열 수 없다
```

`<figgy-chart>`의 공개 API는 다음과 같다.

| 분류 | 메서드와 동작 |
|---|---|
| 객체 수명 | `ready` Promise, `figgy-ready` / `figgy-init-progress` / `figgy-error` / `figgy-select` / `figgy-drag` / `figgy-release` / `figgy-resize` / `figgy-stream-progress` 이벤트, `free()`. 이벤트의 `detail`은 `figgy-ready`가 `{ chart, kernel }`, `figgy-select`가 `{ selected, x, y, originalEvent }`, `figgy-drag`가 `{ x, y, dx, dy }`, `figgy-release`가 `{ selected }`, `figgy-resize`가 `{ width, height, cssWidth, cssHeight, devicePixelRatio }`, `figgy-error`가 `{ error, operation?, recoverable? }`, `figgy-stream-progress`가 `{ executionId, revision, status, submittedPrimitives, totalPrimitives }`이다. 이미 완료된 스트림 화면을 다시 쓰는 `render_chart()`의 `figgy-stream-progress`에는 `status`, `submittedPrimitives`, `totalPrimitives`만 들어 있다. 첫 프레임과 준비 완료 알림 후 백그라운드 피킹 준비를 시작한다. 실패하면 `operation: "prewarm_gpu_picking"`, `recoverable: true`인 `figgy-error`를 보내며 준비 완료 상태는 유지한다. |
| 폰트 | `register_font(Uint8Array)`는 TTF/OTF/TTC를 등록하고 글꼴 이름 배열을 반환한다. 반환된 이름을 `font`에 사용할 수 있다. 등록 폰트가 시스템 폰트보다 우선한다. 내용이 같은 파일은 중복 저장하거나 폰트 세대를 늘리지 않으며 글꼴 ID별 데이터를 재사용한다. 찾을 수 없는 글꼴은 내장 Liberation Sans로 대체한다. 브라우저에서는 설치된 시스템 폰트를 찾지 않으므로 한글을 표시하려면 한글 폰트를 등록해야 한다. 수식은 내장 수식 글꼴로 그리므로 등록이 필요 없고, `rich_text_from_tex(tex, font_size, font, r, g, b, a)`가 TeX에서 `RichText` JSON을 만든다. |
| 자동 배치 | `pkg/figgy.js`의 독립 함수 `autoLayout(configJson)`은 `layout::auto_layout`(#44)을 적용한 설정 JSON을 반환한다. 라벨·제목·범례·색상 막대가 겹치지 않도록 여백, 축 제목 오프셋, 라벨 형식을 정하며 차트는 바꾸지 않는다. `chart.set_config(autoLayout(chart.get_config()))`로 적용하고, 자동 맞춤이나 확대·축소 뒤에 다시 호출한다. |
| 스타일 옵션 | 독립 함수 `draw_style_modes()`는 모드 이름의 JSON 배열을, `draw_style_param_specs(mode)`는 `{key, min, max, default, integer}`의 JSON 배열을 반환한다. UI 슬라이더를 만들 때 이 값을 사용한다. 최소·최대는 권장 범위이며 설정은 바깥 값도 허용하고 렌더러가 안전상 필요한 제한만 적용한다. 기본값은 모델의 `Default`와 같다. |
| 컬럼 등록·교체·제거 | `register_column_f32/f64(id, TypedArray)`는 새 ID를 등록한다. `register_columns_f32/f64(ids, TypedArray, valuesPerColumn)`은 연속된 배열로 새 컬럼을 한 번에 등록하며 전체 성공 또는 전체 실패한다. `update_register_column_f32/f64(id, TypedArray)`는 기존 ID를 매번 업로드해 교체한다. `remove_column(id)`로 제거한다. `register_record(id, fields)`와 `register_ragged(id, values, offsets)`는 레코드와 ragged 컬럼을 등록하거나 교체한다. |
| 시리즈 | `add_line_series(id, x, y, width, label)`은 추가 또는 교체, `remove_series(id)`는 제거다. |
| 범례 | `set_series_label(id, label)`로 텍스트를 바꾼다. 줄바꿈과 유니코드 첨자를 지원하며 빈 문자열이면 해당 행을 지운다. `set_series` / `apply_color_cycle`은 인식 가능한 자동 기호만 갱신하고 사용자 텍스트는 보존한다. 전체 재생성은 `reset_legend_from_series_labels()`를 호출한다. 직접 편집할 때는 `legend.content`의 줄바꿈·탭·고정 너비 기호·구간별 색을 사용한다. 기호의 `field_em`은 2.0em이며 선은 `rule:true`, 점선 패턴은 `rule_dash`로 지정한다. `content.font` / `font_size`와 구간별 설정은 그리기 시 적용한다. |
| 위치 판정 | `hit_test(x, y)`는 요소 ID 또는 `null`을 반환한다. 데이터 영역·축·눈금 라벨·제목·색상 막대와 그 세부 요소를 구분한다. 선택 상태 자체는 바꾸지 않는다. 위치는 렌더러의 레이아웃에서 계산하므로 호스트가 별도로 상자를 관리할 필요가 없다. 등록 요소는 활성 레이어를 따른다. 오버레이가 활성이면 `overlay_<n>:axis_right`, `overlay_<n>:tick_labels_right`, `overlay_<n>:axis_title_right`, `overlay_<n>:legend`가 답하고 기본 Y 축은 답하지 않는다. `"Annotations"`가 활성이고 주석 층이 보이면 그려진 주석 항목이 공유 요소 위에서 `annotation_<n>`으로 답하고, 다른 레이어에서는 클릭이 주석을 통과한다. 저수준 `FiggyChart.decoration_bounds(id)`(`<figgy-chart>`에서는 `element.kernel`로 호출)는 `annotation_<n>`도 받아 그 항목의 선택 상자를 돌려준다. 서브플롯(`{ "Subplot": i }`)이나 그 오버레이(`{ "SubplotOverlay": [i, j] }`)가 활성이면 그 서브플롯 차트의 요소가 `subplot_<i>:` 접두사를 붙인 id(`subplot_0:data_area`, `subplot_0:axis_left`, `subplot_0:overlay_0:axis_right` 등)로 답하고 부모 요소는 답하지 않는다. 드래그·리사이즈는 그 서브플롯의 `config`에 기록된다. |
| 레이어 | `set_active_layer(json)`은 `"Base"`, `{ "Overlay": n }`, `"Annotations"`, `{ "Subplot": i }`, `{ "SubplotOverlay": [i, j] }` JSON으로 상호작용 대상 레이어를 정하고 히트 맵을 그 레이어로 다시 만든다. 레이어가 바뀌면 요소 선택과 주석 선택을 지운다(데이터 선택은 유지). 범위 밖 오버레이·서브플롯 인덱스와 서브플롯 인덱스가 범위 밖인 `SubplotOverlay`는 `"Base"`로, 서브플롯은 있지만 그 오버레이가 없는 `SubplotOverlay`는 해당 `{ "Subplot": i }`로 저장한다. `<figgy-chart>`의 `set_active_layer`는 저장된 레이어를, `active_layer()`는 현재 레이어를 파싱한 값(`"Base"`, `{ Overlay: 0 }` 등)으로 돌려준다. 저수준 `FiggyChart`의 두 메서드는 같은 값을 JSON 문자열로 돌려준다. 활성 레이어는 `auto_fit_all`이 맞출 축도 정한다(「범위 맞춤」 참고). `auto_fit_overlay_y(layer, col, pad)`는 `auto_fit_y`와 같은 규칙으로 오버레이 `layer`의 Y 축을 맞추며, 범위 밖 인덱스는 오류다. 저수준 `FiggyChart.auto_fit_subplot(i, "x" | "y", [cols], pad)`(`<figgy-chart>`에서는 `element.kernel`로 호출)는 서브플롯 축을 맞추며, 링크된 축이면 링크 묶음 전체가 움직인다. 범위 밖 서브플롯 인덱스나 `"x"` / `"y"`가 아닌 축 이름은 오류다. 오버레이와 서브플롯 자체(`overlays`, `subplots` 배열)는 `get_config` / `set_config`로 편집한다. 링크된 축을 바꾸면 같은 호출 안에서 묶음 전체가 움직인다. 그려지는 픽셀은 활성 레이어에 의존하지 않는다. |
| 데이터 피킹 | `pick_point(x, y, max_distance_px)`는 `Promise<{ source_id: string \| null, series_id, point_index, distance_px } \| null>`을 반환한다. 점은 스타일을 반영한 기호 크기, 선은 선분과의 거리로 판정하고 가까운 끝점을 고른다. 오차 막대 몸통·끝선은 대상이 아니다. 좌표는 반환된 인덱스로 원본 컬럼에서 조회한다. 값 라벨(범주형 막대·박스플롯·레이더)을 그리는 차트에서는 포인터 아래의 라벨이 점보다 먼저 답하며, 이때 `point_index`는 범주 행이고 `label_id`(`"series:<시리즈 ID>:label:<행>"`)가 붙는다. `pick_data`는 같은 라벨을 `{ kind: "value_label", series_id, category_index, label_id, distance_px }`로 돌려준다. |
| 전체 사전 준비 | `prewarm_all_with_progress(callback)`은 상주 렌더링·스타일·경로 길이·범위·피킹·등고선용 캐시를 생성하고 `{ scope, stage, phase }`를 알린다. `prewarm_all()`은 콜백 없이 같은 작업을 한다. 둘 다 연결 세대를 확인하는 작업 직렬화 장치를 사용한다. `warm_up()`은 첫 프레임 대기용 호환 메서드다. |
| 피킹 준비 | `prewarm_gpu_picking()`으로 미리 준비하거나 재시도할 수 있다. 저수준 API와 래퍼 모두 렌더러의 같은 파이프라인과 차트 캐시를 사용한다. |
| 범위 맞춤 | `auto_fit_all(pad)`는 활성 레이어의 축만 맞추며, 각 축은 그 축에 그려지는 시리즈(`Config::layer_of_series`)로 맞춘다. `"Base"`와 `"Annotations"`(주석은 기본 축에 그린다)는 부모의 `bottom_x`·`left_y`를, `{ "Subplot": i }`는 서브플롯 `i`의 `bottom_x`·`left_y`를, `{ "Overlay": n }`과 `{ "SubplotOverlay": [i, n] }`은 그 오버레이의 Y 축만 맞춘다. 오버레이는 자기 차트의 X 축을 함께 쓰고 Y 축은 따로 가지므로, 오버레이 시리즈는 그 차트의 X 축 범위에는 들어가고 `left_y` 범위에는 들어가지 않는다. 링크된 축(`link_x`, `link_y`)은 링크 묶음이 함께 쓰는 하나의 축이다. 그래서 묶음에 속한 모든 차트(루트가 부모이면 부모 포함)의 시리즈로 범위를 정하고, 어느 차트가 활성이든 묶음 전체가 그 범위로 움직인다. X 링크와 Y 링크는 따로 따진다. 예를 들어 서브플롯 1의 `link_y`만 서브플롯 0을 가리키면, 둘 중 어느 쪽을 맞춰도 Y 축은 두 서브플롯 시리즈를 합친 범위가 되고 X 축은 각자 자기 시리즈로 맞춘다. 링크되지 않은 축은 자기 차트의 시리즈로만 맞춘다. 모든 레이어를 맞추려면 레이어마다 `set_active_layer`로 바꾼 뒤 `auto_fit_all`을 호출하거나, 컬럼을 지정해 `auto_fit_overlay_y` / `auto_fit_subplot`을 호출한다. 범위는 대상 시리즈의 원본 도형을 모두 포함하며 비율 여백을 더한다(0은 여백 없음, 0.05는 5%). 상주 선·점·오차 막대·행렬·벡터는 GPU로 범위를 계산하며 히스토그램은 경계·빈도 메타데이터와 기준선을 사용한다. 상주 경로는 GPU 결과를 읽어 설정에 반영할 때까지 다른 커널 호출을 막는다. 스트리밍 문서에는 오버레이와 서브플롯이 없으므로 스트리밍 경로는 모든 시리즈로 기본 축을 맞춘다. 래퍼의 스트리밍 경로는 일반 busy 토큰 없이 축 반영과 렌더링 완료를 기다린다. 저수준 스트리밍 호출은 요청만 등록하므로 호스트가 작업을 계속 실행해야 한다. 범위 끝은 반올림하지 않는다. 눈금은 범위 안에서 읽기 좋은 값으로 배치한다. `auto_fit_colorbar(pad)`는 색상 막대가 매핑하는 모든 컬럼(행렬 값, 산점도 `point_color_map`, 점 벡터의 색상 컬럼과 격자 벡터 색상 격자의 모든 컬럼)의 통계로 색상 막대를 맞춘다. 색상 막대가 없는 차트는 바꾸지 않는다. `auto_fit_x/y(col, pad)`는 단일 컬럼 통계를 사용하며 오차 막대를 반영하지 않는다. `load_demo()`는 데모를 불러온다. |
| 행렬·막대 정보 | `set_contour_nice_levels(series_id, target_count, use_colormap_colors)`는 색상 막대 눈금 규칙으로 등고선 값을 만들고 시리즈 설정에 기록한 뒤 레벨 수를 반환한다. `series_draw_info(series_id)`는 `{ drawn_count, cols, rows, truncated }`를 반환한다. 저수준 WASM은 JSON 문자열, 래퍼는 객체를 반환한다. |
| 피킹 기준 | 최종 장식 픽셀 대신 원본 점·선분을 기준으로 한다. 산점도는 지정한 기호 반지름을, 선은 인접 원본 점 사이의 직선을 사용한다. 점선의 공백, 사각 끝 모양, 스케치의 흔들림은 판정 경로에 영향을 주지 않는다. |
| 설정 읽기·쓰기 | `get_config()` / `set_config(json)`, `get_series()` / `set_series(json)`. `set_colorbar_axis(json)`은 색상 막대의 전체 `AxisOptions`를 교체한다. |
| 프리셋 | `apply_axis_preset(AxisPreset)`, `apply_color_cycle(ColorCycle)`, `color_cycle_css(cycle)`. |
| 포인터 입력 | 웹 래퍼가 포인터 이벤트를 처리한다. 직접 전달할 때는 `on_press(x, y)`, `on_move(dx, dy)`, `on_release()`, `has_selection()`을 사용한다. 이 경로는 활성 레이어를 따른다. `"Annotations"`가 활성이면 선택된 주석의 핸들이 먼저 답하고, 그다음 위치의 주석 항목을 선택해(요소 선택은 지움) `on_move`로 끌며, 주석이 없는 곳을 누르면 아래 공유 요소로 넘어가고 주석 선택을 지운다. `has_selection()`은 요소나 주석이 선택돼 있으면 `true`다. 값 라벨은 CPU 히트 맵에 없으므로, 피킹이 돌려준 `label_id`로 `press_value_label(label_id)`를 호출해 선택하고 끌기를 시작한다. 이후 `on_move`가 그 시리즈의 `labels.offsets`를 바꾸고 `on_release`가 끝낸다. `on_press`의 반환값과 `has_selection()`의 차이는 [3.4절](#34-이벤트-입력--포인터를-모델-정책으로-그대로-전달)을 참고한다. |
| 선택 표시 | `set_picked_points(json)`은 `PickedPointsConfig` 또는 `null` JSON 문자열로 `Config.picked_points`만 교체한다. `set_picked_data(json)`는 점·막대·셀·등고선 선택을 같은 방식으로 바꾼다. `null`은 표시를 지운다. |
| 배경 | `set_clear_color(r, g, b, a)`는 선형 RGBA를 성분별 0~1로 제한하고 다시 그리기를 요청한다. 화면 상태만 바꾸므로 Config JSON이나 축 이미지는 갱신하지 않는다. |
| 출력 | `export_png(scale)`은 PNG의 `Uint8Array`를 비동기로 반환한다. 문서 출력이므로 오버레이 레이어와 서브플롯도 함께 그린다. |
| 제목 | `set_title`, `set_x_title`, `set_y_title`, `set_colorbar_title`. 색상 막대 제목에 빈 문자열을 주면 숨긴다. `set_title`은 헬퍼이므로 제목이 제목 영역보다 높으면 `chart_title.top_margin`을 함께 늘린다. 영역에 들어가는 제목은 여백을 바꾸지 않는다. |

점별 스타일은 `set_series(json)` / `get_series()`로 설정하며 전용 WASM 메서드는 없다. 정밀 모드의 산점도는 `point_style_table` / `point_style_index_column` / `point_style_overrides`를, 오차 막대는 `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`를 사용한다. 값 컬럼에 따른 점 색상·크기는 산점도의 `point_color_map` / `point_size_map`으로 지정하며, 색상 매핑에는 `colorbar`가 있어야 한다. 스케치·은하수·별자리 스타일은 이 매핑을 모두 무시한다.

<a id="figgy-chart-kernel"></a>

`element.kernel`로 저수준 WASM `FiggyChart`에 직접 접근할 수 있다. 이 getter는 작업 중이면 `"figgy chart is busy"`, 준비 전이면 준비되지 않았다는 오류를 던진다. 받은 객체로 하는 호출은 웹 래퍼의 작업 중 접근 제한과 수명 관리를 거치지 않는다. [주석 레이어 메서드](#annotation-layer), `decoration_bounds`, `step_selection`, `selection_details`, `auto_fit_subplot`은 `<figgy-chart>`에 전달 메서드가 없으므로 이 경로로 호출한다. 비동기 `selection_details()`가 끝날 때까지 같은 요소의 다른 호출을 막는다. 연결이 끊기거나 `free()`를 호출하면 커널이 해제되므로 받은 객체를 보관하지 말고 호출할 때마다 `element.kernel`로 다시 얻는다.

<a id="exact-streaming"></a>

<a id="공개-후보-자동-실행과-원본-구간-공급"></a>

## 6. 스트리밍 자동 실행과 원본 구간 공급

아래 API는 figgy 0.10.0부터 제공한다. `render_chart()`가 작업 실행을 관리하므로 앱에서 `auto_stream_chart_step()`이나 `frame()`을 반복 호출할 필요가 없다. 네이티브 Rust의 빌린 `ColumnSource` 방식과 청크 실행 API도 계속 사용할 수 있다.

![figgy의 상주 및 비상주 화면 렌더링 구조](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/streaming-architecture-kr.png)

그림은 컬럼 기반 렌더러의 상주 데이터 재사용, 청크 스트리밍과 CPU 래스터 갱신을 화면 갱신 루프 안에서 구분한다. 선택적인 GPU 캐시와 출력 시 원본 재공급은 아래 본문에서 설명한다. 브라우저에서는 다음과 같이 역할을 나눈다.

| 계층 | 관리하는 상태와 역할 |
|---|---|
| 호스트 원본 저장소 | 같은 리비전의 TypedArray나 `readRange` 공급자를 다시 그리기·조회가 끝날 때까지 유지한다. 파일 파싱과 Worker 입출력도 담당한다. |
| 웹 래퍼 | 필요한 구간을 요청·전달하고 rAF, GPU 완료 대기, 취소 신호, 진행 알림과 Promise를 관리한다. 전체 데이터를 따로 복제하거나 처리 위치·통계를 중복 관리하지 않는다. |
| 렌더러 | 설정·시리즈·소스 리비전·처리 위치·청크 통계를 관리한다. 연결된 컬럼 전체를 자동으로 상주 풀에 옮기지 않으며, 현재 화면에 필요한 원본 행을 패킹 캐시에 보관할 수 있는지 판단한다. |
| GPU 렌더링 | 명시적으로 등록한 상주 컬럼은 `ColumnPool`에서 읽는다. 스트리밍은 제한된 청크를 올려 화면 밖 렌더 타깃에 누적한다. 지원되는 차트에서는 예산이 허용할 때 현재 화면의 원본 행만 차트별 패킹 캐시에 유지한다. 데이터를 줄이는 LOD·데시메이션은 적용하지 않는다. |

완료된 패킹 캐시의 점·선 피킹은 GPU 행을 조회하며, 그 밖의 비상주 스트림은 즉시 피킹을 지원하지 않는다. 배율을 지정한 PNG는 원본을 다시 읽어 해당 해상도로 그린다. 따라서 화면 렌더링이 끝나도 크기·DPR 변경이나 이미지 출력에 사용할 원본 공급자를 유지해야 한다. 같은 리비전의 원본은 바꾸지 않으며, 수정하면 새 리비전으로 등록한다. 원본 도형을 빠짐없이 처리하더라도 GPU 백엔드나 렌더 패스에 따라 안티앨리어싱 픽셀값은 달라질 수 있다.

```js
await chart.ready;
// 값은 예시 정책이다. 실제 사용량과 admission 결과에 맞춰 정한다.
chart.configure_streaming(
  1,              // maxActiveCharts
  2,              // maxInFlightChunks (업로드 슬롯 수)
  8,              // maxColumnsPerChunk
  2 * 1024 ** 2,  // maxChunkInputBytes
  16 * 1024 ** 2, // maxInFlightGpuBytes
);
const columns = [
  { id: "x", revision: 1, length: rowCount, encoding: "f64" },
  { id: "y", revision: 1, length: rowCount, encoding: "f32" },
];
chart.register_streaming_column_sources(
  columns.map(c => c.id), columns.map(c => c.revision),
  columns.map(c => c.length), columns.map(c => c.encoding),
);
chart.add_line_series("signal", "x", "y", 1.5, "Signal");
const job = chart.render_chart({
  columns,
  // 원본 저장소의 구간 읽기: 배열 길이는 반드시 length와 같아야 한다.
  readRange: ({ id, revision, offset, length, encoding }) =>
    sourceStore.readRange({ id, revision, offset, length, encoding }),
  maxPrimitivesPerChunk: 262144,
  maxFrameTimeMs: 8,
  stallTimeoutMs: 30000,
  onProgress: progress => updateProgress(progress),
});
// 새 차트이고 표시 범위를 모를 때만 요청한다. 두 호출 사이에 await를 넣지 않는다.
const fitted = chart.auto_fit_all(0.05);
const [result] = await Promise.all([job.done, fitted]);
if (result.status !== "complete") {
  throw new Error(`stream ${result.status}`);
}
const png = await chart.export_png(2);
// 화면을 버릴 때: await job.cancel();
```

### 원본 공급과 작업 실행

- `columns`에는 필요한 수만큼 컬럼을 지정할 수 있다. 전체 배열을 갖고 있다면 `{ id, revision, values: typedArray }`를 전달하고 `readRange`를 생략한다. 웹 래퍼는 배열 참조만 보관하고 필요한 구간만 WASM의 쓰기 전용 스테이징에 기록한다. Worker에서 만든 구간은 전송 가능한 버퍼로 넘길 수 있다. 파일 파싱·원본 저장·Worker 입출력은 호스트가 담당한다.
- 작업이 끝나도 원본 참조나 `readRange`를 유지한다. 크기·DPR 변경과 이미지 출력에 같은 리비전의 원본이 필요하기 때문이다. 같은 리비전의 값을 수정하거나 버퍼를 분리(detach)하지 않는다. 수정하면 새 리비전으로 등록한다. `job.cancel()`이나 컴포넌트 해제 시 해당 작업이 보관하던 참조를 놓는다.
- 설정·시리즈·소스 리비전·처리 위치는 렌더러가 관리한다. 웹 래퍼는 구간 요청과 완료 알림을 연결하며 GPU 계산을 중복 구현하지 않는다.
- `maxFrameTimeMs`는 실제 동기 제출 시간에 맞춰 청크 크기를 조절하기 위한 목표값이다. 브라우저 스케줄링·데이터 공급자·GPU 명령의 실행 시간을 강제로 제한하지는 않는다. 작업은 MessageChannel과 GPU 완료 알림으로 진행하고 화면 표시는 rAF에 맞춘다.
- `job.done`은 `{ status, submittedPrimitives, totalPrimitives }`로 이행된다. `status`는 `"complete"`, `"cancelled"`, `"superseded"` 중 하나이며, 실패하면 Promise가 거부된다. 완료된 패킹 캐시는 `job.done`이 아니라 `stream_status().view_residency.state`의 `"resident"`로 확인한다. `job.cancel()`은 제출한 GPU 작업의 자원 회수까지 기다린다. 실행 중인 GPU 명령을 강제로 중단하지는 않는다. 이전 작업의 데이터 응답이 늦게 와도 새 작업에 반영하지 않는다.
- 일반 점·선의 원본 구간 업로드는 GPU 작업 버퍼와 제한된 수의 staging 슬롯을 재사용한다. 슬롯 수는 `configure_streaming`의 두 번째 인자(`maxInFlightChunks`)를 넘지 않으며, GPU 복사와 다시 매핑하는 작업이 끝나지 않았으면 `backpressure`로 기다린다. 호스트는 GPU 완료를 기다리고 브라우저 이벤트 루프에 실행 기회를 줘야 한다. 초기 fit으로 다시 그릴 때도 슬롯을 유지한다. 선택 표시·격자·경로 길이 계산·export의 별도 업로드 자원에는 같은 재사용 방식을 일괄 적용하지 않는다.
- 취소·교체 등으로 마지막 공유 소유자가 자원을 놓으면 회수 대기 상태로 옮긴다. 기존 명령의 제출 경계를 지난 뒤 GPU 완료를 확인하고 추적 중인 버퍼·텍스처에 `destroy()`를 호출한다. 사용 중인 버퍼를 청크마다 파괴하는 동작이 아니며, JavaScript GC가 실행될 때까지 반환을 미루지 않기 위한 처리다.
- `stream_status()`는 상태만 조회한다. 완료 후에도 누적 도형 수·리비전·작업 ID를 유지한다. 같은 입력을 다시 요청하면 원본을 다시 읽거나 그리지 않는다.

<a id="streaming-fit"></a>

### 첫 스트림의 범위 맞춤과 이후 조작

**스트리밍 자체는 자동 맞춤을 켜지 않는다.** 새 차트의 표시 범위를 모르면 호스트가 첫 청크 전에 한 번 요청한다. 렌더러는 원본 구간을 앞으로 읽으면서 누적 범위를 계산한다. 새 청크로 범위가 넓어지면 앞서 그린 데이터 이미지를 GPU에서 리스케일하고 새 청크를 이어 그린다. 축과 눈금은 새 범위로 따로 그리므로 이전 눈금이 이미지와 함께 늘어나거나 잔상으로 남지 않는다.

숫자 눈금 라벨은 현재 축 범위에서 실제 글자 크기와 화면 길이를 재어, 일정한 간격으로 표시한다. 화면을 넓히거나 확대하면 생략했던 숫자가 다시 나타난다. 글꼴 크기와 설정의 주 눈금·격자 간격은 유지하며, 지수 표기·반전 축·색상 막대에도 같은 규칙을 적용한다. PNG 배율에 맞춰 글자 사이 여백도 함께 커진다.

초기 fit의 중간 화면은 미리보기다. 리스케일 때문에 점 크기·선 두께·선명도가 잠시 달라질 수 있다. 전체 범위가 확정되면 원본을 **한 번만** 다시 읽어 정밀하게 그린다. 이 2순회에서도 새 청크가 반영된 화면 구역부터 갱신하고, 나머지 구역은 1순회 미리보기를 유지한다. 화면 전체를 지우거나 끝날 때까지 표시를 멈추지 않는다. 두 이미지를 덧그리지 않고 픽셀마다 하나를 선택하므로 반투명 중첩이나 AA가 이중으로 합성되지 않는다. 글씨·축·선택 표시는 별도로 한 번만 그린다.

변경된 픽셀만 교체하며, 아직 처리하지 않은 선의 연결부가 지워지지 않도록 옆 픽셀까지 교체하지 않는다. 아직 처리하지 않은 도형이 같은 픽셀에 겹치면 중간 모습은 잠시 달라질 수 있다. 완료 시에는 정밀 누적 결과만 사용하며, 최종 화면과 PNG는 리스케일된 이미지를 확대해서 만드는 결과물이 아니다. 부분 갱신에는 화면 크기·MSAA에 비례하는 임시 텍스처 두 장과 작은 구역 버퍼가 필요하다. 전체 GPU 예산에 여유가 없으면 기존처럼 미리보기를 유지하다가 완료 후 교체한다. 원본 데이터 크기나 청크 수에 따라 메모리가 계속 늘어나지는 않는다.

일반 점·선은 최초 순방향 처리와 마지막 정밀 렌더링으로 전체 구간을 두 번 읽는다. 청크 경계의 인접 값과 스타일별 GPU 계산에 필요한 읽기는 별도다. 범위가 바뀔 때마다 읽기 커서를 0으로 되돌리는 반복은 하지 않는다. 뷰 패킹 캐시는 마지막 정밀 렌더링에서 만든다.

`submitted_primitives`는 초기 순방향 처리의 진행량을 유지하며 마지막 정밀 렌더링 중에도 되감기지 않는다. 값이 `total_primitives`에 도달한 것만으로 완료로 판단하지 않는다. 최종 화면과 설정이 확정된 `complete` 또는 웹 래퍼의 `job.done`을 기다린다. 확정 설정에는 축 범위와 그 범위에 맞는 눈금 간격·표기 형식이 함께 반영된다.

| 상황 | 호출 방법 | 결과 |
|---|---|---|
| 범위를 모르는 새 차트 | 첫 청크 전에 자동 맞춤 요청 | 처리한 청크까지의 범위로 계속 맞추고, 완료 시 최종 범위 확정 |
| 저장한 차트 열기·수동 범위로 시작하기 | 설정을 복원하고 렌더링만 요청 | 저장되거나 지정된 범위 유지 |
| 확대·축소·이동 후 다시 그리기 | 범위를 바꾸고 렌더링만 요청 | 사용자가 정한 범위 유지 |
| 전체 데이터를 다시 보고 싶을 때 | `auto_fit_all(padding)` 재호출 | 현재 연결된 전체 시리즈에 맞춤 |
| 과거에 저장한 범위로 정확히 돌아가기 | 저장해 둔 축 범위를 설정에 복원 | 당시 범위 복원 |

진행 중인 자동 맞춤 요청은 요청 당시의 설정과 시리즈에 속한다. `set_config()`·`set_series()`로 설정이나 시리즈를 복원하거나 작업을 취소하면 이전 요청을 해제한다. 같은 WASM 객체로 다른 차트를 표시해도 이전 차트의 맞춤 요청을 이어받지 않는다. 다시 맞춰야 한다면 설정과 시리즈를 모두 적용한 뒤 명시적으로 요청한다.

창 크기나 DPR이 바뀌면 실제 캔버스 픽셀 크기와 `resize(width, height)`를 함께 갱신한다. 저수준 스트리밍 호스트는 `request_auto_streaming_chart()`를 다시 호출해 새 크기의 표시 타깃을 요청하고 완료까지 진행한다. 이때 `auto_fit_all()`은 호출하지 않는다. 저장된 축 범위와 문서 크기는 유지하고, 글자·테두리·최종 데이터를 새 해상도로 그린다. 작은 캔버스를 CSS로 늘린 상태를 최종 화면으로 사용하지 않는다.

자동 맞춤은 현재 데이터로 범위를 계산하는 동작이다. 데이터와 여백이 같으면 처음 맞춘 범위로 돌아가지만, 데이터가 바뀌었다면 새 데이터에 맞춘다. `padding`은 유한한 0 이상의 수이며, `0`은 여백 없음, `0.05`는 5% 여백이다. 범위 계산과 통계 재사용은 렌더러가 담당한다.

현재 `Config`의 축 `min`·`max`는 숫자다. 기본값만 보고 새 차트인지, 사용자가 그 범위를 선택했는지 구분할 수 없으므로 생성·불러오기 흐름에서 자동 맞춤 여부를 정한다. 이 구분을 위해 `null`이나 생략한 범위를 전달하지 않는다.

**웹 래퍼(`<figgy-chart>`)**에서는 위 예제처럼 `render_chart()`로 원본 공급자를 연결한 직후, 같은 동기 호출 구간에서 `auto_fit_all()`을 호출한다. `await job.done` 뒤에 요청하면 첫 렌더링이 기존 범위로 진행된다. 반대로 `readRange` 공급자를 연결하기 전에 맞춤을 요청하면 다시 공급할 원본이 없으므로 `auto_fit_all()`이 반환한 Promise가 거부된다. 두 Promise를 함께 기다리고, 취소·대체된 작업의 결과를 현재 차트에 반영하지 않는다.

확대·이동이 끝난 뒤 전체 데이터를 다시 보려면 원본 공급자를 유지한 상태에서 다음과 같이 호출한다. 이 호출은 범위 반영과 렌더링 완료까지 기다린다.

```js
await chart.auto_fit_all(0.05);
const fittedConfig = JSON.parse(chart.get_config());
// 속성 패널·저장 기능은 렌더러가 확정한 fittedConfig를 사용한다.
```

**저수준 WASM(`FiggyChart`)**은 실행 순서가 다르다. 컬럼과 시리즈를 등록한 뒤 `await chart.auto_fit_all(padding)`으로 요청을 등록하고, `request_auto_streaming_chart(maxPrimitivesPerChunk)`를 호출한다. 이후 호스트가 `auto_stream_chart_step(...)`과 `frame()`을 계속 실행한다. 이 경로의 스트리밍 `auto_fit_all()`은 요청 등록까지만 기다리므로 반환 직후의 설정을 최종 맞춤 결과로 저장하면 안 된다. GPU 완료를 기다리며 `complete`까지 진행한 뒤 `get_config()`를 읽는다. `all_submitted`는 제출 완료이며 최종 화면 반영 완료와 다르다.

**네이티브 Rust의 자동 스트리밍**은 등록된 차트에 아래 순서로 요청한다. `renderer`, `chart_id`, `view`, `options`는 호스트가 준비한 기존 객체다.

```rust
renderer.request_stream_auto_fit(chart_id, 0.05)?;
renderer.request_auto_streaming_chart(chart_id, &view, options)?;
```

이후 호스트의 기존 실행 루프에서 `auto_stream_chart_step`으로 요청된 원본을 공급하고, `prepare_registered`와 그리기로 중간 결과를 표시한다. GPU 완료도 처리하며 `Complete`까지 진행한 뒤 `chart_config(chart_id)`로 확정된 범위를 읽는다. `request_stream_auto_fit`은 자동 스트리밍용 요청이며, 명시적 청크 실행 API나 상주 렌더링의 범위를 바꾸는 호출은 아니다. 이후 사용자가 범위를 바꿔 다시 그릴 때는 이 요청을 반복하지 않는다.

첫 맞춤이 끝난 뒤에도 같은 리비전의 원본을 다시 공급할 수 있어야 한다. 범위가 넓어졌을 때의 재그리기, 자동 맞춤 재요청, PNG 출력에 필요하다. JavaScript에서 청크별 최솟값·최댓값을 별도로 계산하거나 렌더러의 확정 범위를 매 프레임 덮어쓰지 않는다.

### 화면 변경, 선택과 출력

- 데이터나 표시 범위가 바뀌면 다음 작업 단계에서 최신 상태로 교체한다. 제목 같은 장식만 바뀌면 데이터 처리 위치와 누적 이미지를 유지한다. 크기·DPR 변경 시에는 새 픽셀 해상도로 원본을 다시 그린다.
- `await auto_fit_all()`은 통계로 계산한 축 범위의 반영과 렌더링 완료를 기다린다. 일반 비동기 호출의 `busy` 토큰을 잡지 않으므로 스트리밍 실행은 계속 진행된다. 통계는 최초 구간 업로드 때 수집하고 렌더러가 리비전별로 재사용한다.
- 첫 스트림과 자동 맞춤 버튼의 호출 순서는 [범위 맞춤 사용법](#streaming-fit)을 따른다. 일반 재그리기는 자동 맞춤을 켜지 않는다.
- 선택만 바뀌면 데이터 스트림을 다시 시작하지 않는다. 선택한 원본 구간만 요청하고 새 GPU 자원이 준비될 때까지 이전 표시를 유지한 뒤 한 번에 교체한다. 공급이 실패해도 이전 표시를 유지하고 `figgy-error`로 알린다. 취소되거나 다른 선택으로 대체된 요청의 지연 응답은 무시한다. 출력은 시작 당시의 선택을 사용하므로 이후 화면 선택과 섞이지 않는다.
- 완료된 패킹 캐시가 있으면 `pick_point` / `pick_data`가 GPU의 점·선에서 원본 행 인덱스를 반환한다. 원본을 다시 읽지 않는다. `next_view_point_index(sourceId, seriesId, current, forward)`는 캐시에 남아 있는 행 중 다음·이전 원본 인덱스를 동기적으로 반환하며, 대상이 없으면 `null`이다. 이 조회로 GPU 피킹이나 공급자 요청을 다시 실행하지 않는다.
- 패킹 캐시가 없거나 아직 준비 중인 비상주 스트림의 피킹은 즉시 `null`을 반환한다. 저수준 WASM의 `begin_stream_pick_*` / `finish_stream_pick_*`는 호환성을 위해 함수 형식만 남겨 두며, 반환하는 Promise는 즉시 거부된다. 상주 차트의 피킹은 계속 지원한다. 이미 아는 인덱스의 선택 표시는 필요한 원본 구간만 읽어 처리한다.
- `export_png`는 문서 크기와 출력 배율에 맞춰 원본을 다시 그린다. 화면 텍스처를 늘리지 않으며 화면 작업의 처리 위치나 누적 이미지는 바꾸지 않는다.
- `export_svg` / `export_pdf`는 스트리밍으로 표시 중인 차트를 거부한다. 상주 차트에서만 사용할 수 있다.

### GPU 캐시와 메모리 한도

- `inspect_column_admission(metadata)`는 전체 컬럼을 명시적으로 등록할 때 필요한 인코딩 크기·한도·거부 이유를 조회한다. 공간을 예약하거나 모든 파생 자원의 할당을 보장하는 호출은 아니며 자동 스트리밍의 실행 기준도 아니다.
- `configure_auto_residency(memoryBudgetBytes, workingSetLimitBytes)`는 전체 GPU 예산과 차트별 GPU 캐시 한도를 바이트 단위로 정한다. 예산이 현재 렌더러 사용량보다 작으면 오류를 반환한다. 지원되는 정밀 모드의 점·실선·오차 막대 차트는 스트리밍 중 현재 화면에 필요한 원본 행만 GPU에 남긴다. 완성된 패킹 페이지는 GPU로 옮기고 CPU에는 현재 청크와 작성 중인 페이지만 둔다. 화면 밖 점을 잇지만 화면을 통과하는 선과 화면에 닿는 오차 막대의 원본 행도 포함한다. 상주 한도 0은 패킹 캐시를 끈다. 연결된 컬럼 전체를 자동으로 `ColumnPool`에 넣지는 않는다.
- 표시 범위를 좁히거나 창 크기·비율·DPR을 바꿔도, 새 화면의 필요 영역이 캐시에 있으면 패킹된 원본 행만으로 다시 그린다. 캐시에는 선 두께·점 크기·오차 막대 끝과 AA 영역에 더해 가장자리 8픽셀의 여유를 포함한다. 이 추가 데이터도 기존 패킹 한도에 포함된다. 화면 크기의 일치 여부 대신 데이터 좌표에서 실제 포함 관계를 검사하며, 새로 필요한 가장자리까지 캐시가 덮지 못하면 원본 구간을 다시 요청한다. 지원하지 않는 도형, 패킹 한도 초과, GPU 예산 부족, 할당 실패 시에는 원본을 빠짐없이 그리는 스트리밍 경로를 유지한다. `stream_status().view_residency`의 `state`, `needed_bytes`, `refusal_reason`, `picking_available`로 상태를 확인한다. `needed_bytes`는 준비 중에는 지금까지 필요한 최소 바이트 수, 완료 후에는 실제 패킹 크기다. `job.cancel()`은 해당 작업과 표시 자원을 정리한다.
- 점선·히스토그램·히트맵·스케치·별자리와 지원되는 은하수 스트림에는 원본 순서와 인접 관계를 보존하는 별도 GPU 캐시를 사용한다. 참조 컬럼 전체의 hi/lo 인코딩 크기 합계가 차트별 상주 한도와 장치의 단일 버퍼 한도에 들어오고 전체 GPU 예산에도 여유가 있을 때만 만든다. 청크를 올릴 때 GPU 안에서 복사하며 CPU 원본 복사본은 남기지 않는다. 크기·DPR·축 범위·그리드·히트맵 색상 매핑 변경은 캐시된 원본으로 다시 계산하므로 이미지 확대에 따른 흐림이나 이전 설정의 잔상이 생기지 않는다. 캐시에 없는 구간, 새 데이터 리비전, 예산 초과는 일반 스트리밍으로 처리한다. 큰 컬럼의 일부만 잘라 이 캐시에 넣지는 않는다. 원본 공급자는 출력과 캐시 미적중에 대비해 계속 보관해야 한다.
- 원본 구간 캐시는 데이터 재공급 비용을 줄이는 기능이다. `view_residency`는 기존의 패킹된 뷰와 즉시 피킹 가능 여부를 나타내므로 이 캐시만 있는 차트는 여전히 `streamed`로 표시한다. 상주 한도를 0으로 내리면 원본 구간 캐시도 해제한다. 네이티브 `auto_stream_chart_request_ranges()`는 캐시 적중 시 한 청크의 GPU 작업을 진행하고 `Backpressure`를 반환할 수 있다. 기존처럼 완료까지 계속 호출하고 `Ready`일 때만 원본을 공급한다. 조회만 필요하면 작업을 진행하지 않는 `stream_status()`를 사용한다.
- 웹 컬럼 풀은 16MiB로 시작하고, 업로드 공간이 부족하면 자동으로 늘어난다. 기본값은 `true`이며 장치의 스토리지 바인딩 한도와 전체 GPU 예산을 넘지 않는다. `set_pool_auto_growth(false)`로 끄면 현재 크기를 유지한다. 할당 오류는 JS `Error.code`의 `pool_space`, `budget_exceeded`, `device_limit`, `allocation_failed`로 구분한다. 가능한 경우 `requestedBytes`, `limitBytes`, `largestFreeBytes`, `totalFreeBytes`도 제공한다. JavaScript가 정수로 정확히 표현할 수 있는 범위 안에서는 숫자, 그 밖에서는 10진 문자열로 반환한다.
- `gpu_memory_status()`는 요청 할당량의 사용 중·회수 대기·합계(`live/retired/total`), 풀 용량·점유·최대 빈 공간·백업·반환 대기량, 자원별 사용량, 컬럼별 `resident` / `streamed` 상태를 반환한다. 실제 VRAM 잔량을 측정하지는 않는다. 은하수·별자리 스타일의 미리 생성한 텍스처는 일반 집계에서 제외되며 스트리밍 출력의 실행 가능 여부를 검사할 때만 크기를 따로 고려한다.
- `await release_unused_gpu_memory()`는 활성 스트림이 없을 때 남은 컬럼을 GPU 안에서 작은 풀로 옮기고 이전 버퍼의 작업 완료를 기다린다. 최소 16MiB는 유지한다. 축소용 버퍼도 잠시 필요하므로 예산·장치 한도·할당 오류가 발생할 수 있다. 실패하면 기존 컬럼과 바인딩을 유지한다.
- 호스트 정책의 예로 전체 예산 `4_000_000_000`바이트, 뷰별 패킹 한도 `500_000_000`바이트를 둘 수 있다. 현재 화면의 캐시가 한도를 넘으면 스트리밍을 유지한다. 컬럼 등록 시 공간·예산이 부족하면 불필요한 컬럼 제거, 정리 완료 대기, 한 번 재시도 순서로 처리한다. 계속 실패하거나 장치 한도·실제 할당 오류가 나면 스트리밍으로 전환한다. 이 수치는 예시이며 렌더러의 기본값이 아니다.

정밀 스타일의 스트림 누적 화면에는 데이터만 저장한다. 그리드는 표시할 때 아래에 합성하므로 표시 여부·색·두께·선 모양을 바꿔도 작업 ID와 데이터 처리 위치를 유지한다. 상주 캐시가 없는 스트림도 원본 재생 없이 그리드를 갱신한다. 배경에 의존하는 다른 스타일은 그리드를 바꾸면 데이터를 다시 그린다. 원본 구간 GPU 캐시가 있으면 호스트에 같은 청크를 다시 요청하지 않는다. 정밀 스타일의 분리 합성은 텍스처를 리샘플링하거나 AA를 다시 적용하는 과정이 아니다. 다만 UNORM 색상 합성과 MSAA 반올림 때문에 배경 위에 직접 그리는 상주 경로와 색상 채널이 비트 단위로 같다는 보장은 없다. 회귀 장면에서는 채널 차이를 8비트 값 기준 최대 2로 제한하며, 반투명 점·막대 채움·테두리가 겹치는 장면은 최대 3으로 검사한다.

`crates/web/tests/view-cache-resize-grid-probe.html`은 창 크기·비율 변경 시 원본 요청이 없는지, 새 크기에서 처음부터 스트리밍한 화면과 픽셀이 일치하는지, 그리드 변경이 작업 재시작 없이 반영되는지 검사한다.

### 지원 범위

- 비상주 히트맵은 그리기·선택 표시·출력을 지원하지만 즉시 피킹은 지원하지 않는다. 이미 아는 셀을 표시할 때는 경계 계산에 필요한 인접 축 값만 읽는다. 자동 범위 맞춤은 기존 GPU 격자 계산을 사용하고 결과를 리비전별로 저장한다.
- `streaming_capabilities()`는 현재 설정과 시리즈의 지원 여부·이유를 반환한다. `operations_require_completed_revision`은 조회·출력 전에 작업 완료가 필요한지를 나타낸다. 지원하지 않는 조합은 무시하거나 다른 스타일로 바꾸지 않는다. 등고선과 은하수의 스트리밍 선·별 연결은 지원하지 않는다. 극좌표 차트, 영역 채우기·계단선, 벡터 시리즈, 점 색상·크기 매핑, 오버레이 레이어와 서브플롯도 상주 컬럼이 필요하므로 스트리밍 원본을 연결하면 오류를 반환한다. 정밀·스케치·별자리의 점선은 기존 경로 길이 스캔의 연산 순서를 유지하므로 청크가 바뀌어도 점선 간격이 처음부터 다시 시작되지 않는다.

브라우저 회귀 검증 페이지는 `crates/web/tests/streaming-contract-probe.html`이다. 430만 점, Worker 구간 공급, 상주 차트 동시 표시, 크기 변경·취소·범위 맞춤, 패킹 캐시 없는 스트림 피킹의 즉시 종료와 원본 무조회, 1배·2배 출력을 검사한다.

저수준 `pick_point`는 JSON 문자열 또는 `undefined`로, 웹 래퍼의 `pick_point`는 객체 또는 `null`로 이행하며, 오류는 Promise 거부로 전달한다. 제출된 요청은 결과 읽기 자원과 당시의 `Arc` 기반 `source_id` / `series_id` 매핑을 보관한다. 대기 중 차트·풀이 바뀌거나 렌더러가 해제돼도 해당 요청의 대상은 바뀌지 않는다.

<a id="streaming-contract"></a>

<a id="exact-streaming-en"></a>

## Streaming contract (English)

The API below has been available since figgy 0.10.0. `render_chart()` runs the job, so the application does not call `auto_stream_chart_step()` or `frame()` repeatedly. Native Rust hosts can still use borrowed `ColumnSource` inputs and the chunk execution API.

![figgy resident and nonresident display rendering architecture](https://raw.githubusercontent.com/shim9610/figgy/807327081b97d20964cc96ce543dfb93a27499be/crates/renderer/assets/streaming-architecture-en.png)

Inside the screen update loop, the figure separates resident data reuse, chunked streaming and CPU raster updates for the column-based renderer. The optional GPU cache and the source replay for export are described below. In the browser the roles are divided as follows.

| Layer | State it owns and its role |
|---|---|
| Host source store | Keeps the TypedArray or `readRange` provider of the same revision until redraws and queries are finished. It also handles file parsing and Worker I/O. |
| Web wrapper | Requests and forwards the needed ranges, and manages rAF, GPU completion waits, cancellation signals, progress notifications and Promises. It does not copy the whole data set or track the processing position or statistics a second time. |
| Renderer | Owns the configuration, series, source revisions, processing position and chunk statistics. It does not move connected columns into the resident pool automatically; it decides whether the original rows the current view needs can be kept in the packed cache. |
| GPU rendering | Explicitly registered resident columns are read from `ColumnPool`. Streaming uploads bounded chunks and accumulates them in an offscreen render target. For supported charts, when the budget allows, only the current view's original rows stay in a chart-local packed cache. No LOD or decimation reduces the data. |

Point and line picking on a completed packed cache looks up GPU rows; other nonresident streams have no immediate picking. A PNG at a given scale rereads the original data and draws it at that resolution. Keep the source provider after on-screen rendering finishes, because size and DPR changes and image export use it. Do not change the data of a revision; register changed data as a new revision. Even when every original primitive is processed, antialiased pixel values can differ between GPU backends and render passes.

```js
await chart.ready;
// The values are an example policy. Choose them from actual usage and admission results.
chart.configure_streaming(
  1,              // maxActiveCharts
  2,              // maxInFlightChunks (number of upload slots)
  8,              // maxColumnsPerChunk
  2 * 1024 ** 2,  // maxChunkInputBytes
  16 * 1024 ** 2, // maxInFlightGpuBytes
);
const columns = [
  { id: "x", revision: 1, length: rowCount, encoding: "f64" },
  { id: "y", revision: 1, length: rowCount, encoding: "f32" },
];
chart.register_streaming_column_sources(
  columns.map(c => c.id), columns.map(c => c.revision),
  columns.map(c => c.length), columns.map(c => c.encoding),
);
chart.add_line_series("signal", "x", "y", 1.5, "Signal");
const job = chart.render_chart({
  columns,
  // Range read from the source store: the array length must equal length.
  readRange: ({ id, revision, offset, length, encoding }) =>
    sourceStore.readRange({ id, revision, offset, length, encoding }),
  maxPrimitivesPerChunk: 262144,
  maxFrameTimeMs: 8,
  stallTimeoutMs: 30000,
  onProgress: progress => updateProgress(progress),
});
// Request only for a new chart whose display range is unknown. Do not put an await between the two calls.
const fitted = chart.auto_fit_all(0.05);
const [result] = await Promise.all([job.done, fitted]);
if (result.status !== "complete") {
  throw new Error(`stream ${result.status}`);
}
const png = await chart.export_png(2);
// To discard the display: await job.cancel();
```

### Supplying sources and running jobs

- `columns` can list as many columns as needed. When the whole array is in memory, pass `{ id, revision, values: typedArray }` and omit `readRange`. The web wrapper keeps only the array reference and writes only the needed ranges into the write-only WASM staging. Ranges produced in a Worker can be passed as transferable buffers. The host owns file parsing, source storage and Worker I/O.
- Keep the source references or `readRange` after the job finishes: size and DPR changes and image export need the source of the same revision. Do not modify the values of a revision or detach its buffers; register changed data as a new revision. `job.cancel()` or disposing of the component releases the references the job held.
- The renderer owns the configuration, series, source revisions and processing position. The web wrapper connects range requests with completion notifications and does not reimplement GPU computation.
- `maxFrameTimeMs` is a target for sizing chunks from the measured synchronous submission time. It does not enforce a time limit on browser scheduling, the data provider or GPU command execution. Work advances through MessageChannel tasks and GPU completion notifications; on-screen updates follow rAF.
- `job.done` fulfils with `{ status, submittedPrimitives, totalPrimitives }`. `status` is `"complete"`, `"cancelled"` or `"superseded"`; on failure the Promise rejects. A completed packed cache is reported as `"resident"` in `stream_status().view_residency.state`, not by `job.done`. `job.cancel()` waits until the resources of the submitted GPU work are reclaimed. It does not force running GPU commands to stop. A late data response for an earlier job is never applied to a newer one.
- Original-range uploads for ordinary points and lines reuse the GPU work buffer and a bounded number of staging slots. The slot count never exceeds the second argument of `configure_streaming` (`maxInFlightChunks`); while GPU copies and remapping are unfinished, the next upload waits and the step reports `backpressure`. The host must wait for GPU completion and give the browser event loop a chance to run. The slots are also kept when the initial fit redraws. The separate upload resources for selection highlights, grids, path-length computation and export do not all use the same reuse scheme.
- When cancellation, replacement or a similar change makes the last shared owner release a resource, the resource moves to the retirement queue. After the submission boundary of earlier commands, GPU completion is confirmed and `destroy()` is called on the tracked buffers and textures. This does not destroy buffers in use on every chunk; the explicit `destroy()` keeps the return of GPU memory from waiting for JavaScript garbage collection.
- `stream_status()` only reads state. After completion it keeps the accumulated primitive count, revision and job ID. Requesting the same input again neither rereads nor redraws the source.

<a id="streaming-fit-en"></a>

### Fitting the first stream and later interaction

**Streaming does not enable auto-fit by itself.** When the display range of a new chart is unknown, the host requests one fit before the first chunk. The renderer computes the accumulated range while reading the original ranges forward. When a new chunk widens the range, the data image drawn so far is rescaled on the GPU and the new chunk is drawn after it. Axes and ticks are drawn separately for the new range, so old ticks are not stretched with the image or left behind as ghosts.

Numeric tick labels are placed at an even interval chosen from the measured text size and the on-screen axis length in the current axis range. Widening the canvas or zooming in brings back numbers that were omitted. The font size and the configured major tick and grid spacing are kept, and the same rule applies to exponent notation, inverted axes and colour bars. The gaps between labels grow with the PNG scale.

The intermediate display of the initial fit is a preview. Rescaling can briefly change point size, line width and sharpness. Once the full range is known, the source is reread **once** and drawn exactly. During this second pass, screen regions reached by new chunks are updated first and the remaining regions keep the first-pass preview. The display is never cleared and never paused until the end. Each pixel takes one of the two images instead of drawing one over the other, so translucent overlaps and AA are never composited twice. Text, axes and selection highlights are drawn separately, once.

Only changed pixels are replaced. Neighbouring pixels are not replaced, so the unprocessed continuation of a line is not erased. Where unprocessed primitives overlap the same pixel, the intermediate appearance can briefly differ. On completion only the exact accumulation is used; the final display and the PNG are not made by enlarging the rescaled image. Partial updates need two temporary textures proportional to the screen size and MSAA, plus a small region buffer. When the overall GPU budget has no room for them, the preview stays until completion and is then replaced. Memory does not keep growing with the source size or the number of chunks.

Ordinary points and lines read the full range twice: in the first forward pass and in the final exact rendering. Reads of neighbouring values at chunk boundaries and reads for style-specific GPU computation come on top of these two passes. The read cursor is not reset to 0 each time the range changes. The packed view cache is built during the final exact rendering.

`submitted_primitives` keeps the progress of the initial forward pass and does not rewind during the final exact rendering. Reaching `total_primitives` alone does not mean completion. Wait for `complete`, where the final display and configuration are committed, or for the web wrapper's `job.done`. The committed configuration includes the axis range together with the tick spacing and number format that fit it.

| Situation | How to call | Result |
|---|---|---|
| New chart with an unknown range | Request auto-fit before the first chunk | Keeps fitting to the range of the chunks processed so far, and commits the final range on completion |
| Opening a saved chart or starting from a manual range | Restore the configuration and request rendering only | Keeps the saved or specified range |
| Redrawing after zooming in or out or panning | Change the range and request rendering only | Keeps the range the user chose |
| Showing all data again | Call `auto_fit_all(padding)` again | Fits all currently connected series |
| Returning exactly to a range saved earlier | Restore the saved axis range into the configuration | Restores that range |

A pending auto-fit request belongs to the configuration and series current when it was made. Restoring the configuration or the series with `set_config()` or `set_series()`, or cancelling the job, releases the earlier request. Showing another chart with the same WASM object does not inherit the previous chart's fit request. To fit again, apply both the configuration and the series, then request the fit explicitly.

When the window size or DPR changes, update the actual canvas pixel size and call `resize(width, height)` together. A raw streaming host calls `request_auto_streaming_chart()` again to request a display target of the new size and runs it to completion. Do not call `auto_fit_all()` at this point. The saved axis range and document size are kept, and text, borders and the final data are drawn at the new resolution. A small canvas stretched with CSS is never used as the final display.

Auto-fit computes the range from the current data. With the same data and padding it returns to the range fitted first; if the data changed, it fits the new data. `padding` is a finite number of at least 0: `0` means no padding and `0.05` means 5% padding. The renderer computes the range and reuses the statistics.

Axis `min` and `max` in the current `Config` are numbers. The default values cannot tell a new chart from a range the user chose, so the create or load flow decides whether to auto-fit. Do not pass a `null` or omitted range to express this.

In the **web wrapper (`<figgy-chart>`)**, call `auto_fit_all()` right after connecting the source provider with `render_chart()`, in the same synchronous section, as in the example above. Requesting it after `await job.done` makes the first rendering run with the existing range. Requesting the fit before a `readRange` provider is connected leaves no source to replay, so the Promise returned by `auto_fit_all()` rejects. Await both Promises together, and do not apply the results of cancelled or superseded jobs to the current chart.

To show all data again after zooming or panning, keep the source provider and call the following. The call waits until the range is applied and rendering completes.

```js
await chart.auto_fit_all(0.05);
const fittedConfig = JSON.parse(chart.get_config());
// Property panels and save features use fittedConfig, as committed by the renderer.
```

**Raw WASM (`FiggyChart`)** uses a different call order. After registering the columns and series, register the request with `await chart.auto_fit_all(padding)` and call `request_auto_streaming_chart(maxPrimitivesPerChunk)`. The host then keeps running `auto_stream_chart_step(...)` and `frame()`. On this path the streaming `auto_fit_all()` waits only until the request is registered, so the configuration read right after it returns must not be saved as the final fit. Wait for GPU completion, run to `complete`, and then read `get_config()`. `all_submitted` means that submission is complete, which is not the same as the final display being applied.

For **native Rust automatic streaming**, the host makes the following requests on a registered chart, in this order. `renderer`, `chart_id`, `view` and `options` are existing objects the host prepared.

```rust
renderer.request_stream_auto_fit(chart_id, 0.05)?;
renderer.request_auto_streaming_chart(chart_id, &view, options)?;
```

The host's existing run loop then supplies the requested source through `auto_stream_chart_step` and shows intermediate results with `prepare_registered` and drawing. It also processes GPU completion, runs to `Complete`, and then reads the committed range with `chart_config(chart_id)`. `request_stream_auto_fit` is a request for automatic streaming; it does not change the range of the explicit chunk execution API or of resident rendering. Do not repeat this request when the user later changes the range and redraws.

After the first fit, the source of the same revision must remain available: redrawing after the range widens, a new auto-fit request and PNG export need it. Do not compute per-chunk minima and maxima separately in JavaScript, and do not overwrite the renderer's committed range every frame.

### View changes, selection and export

- When the data or the display range changes, the next job step replaces the work with the latest state. When only decorations such as the title change, the data processing position and the accumulation image are kept. On size or DPR changes the source is redrawn at the new pixel resolution.
- `await auto_fit_all()` waits until the axis range computed from statistics is applied and rendering completes. It does not take the `busy` token of ordinary async calls, so streaming keeps running. Statistics are collected during the first range upload and reused per revision by the renderer.
- For the call order of the first stream and of an auto-fit button, see [fitting the first stream](#streaming-fit-en). Ordinary redraws do not enable auto-fit.
- A selection-only change does not restart the data stream. Only the selected original ranges are requested; the previous highlight stays until the new GPU resources are ready and is then replaced at once. If supplying the ranges fails, the previous highlight stays and a `figgy-error` event reports the failure. Late responses to cancelled or superseded requests are ignored. Export uses the selection current when it started, so later on-screen selections do not mix into it.
- With a completed packed cache, `pick_point` / `pick_data` return original row indices from the points and lines on the GPU, without rereading the source. `next_view_point_index(sourceId, seriesId, current, forward)` synchronously returns the next or previous original index among the rows in the cache, or `null` when there is none. This lookup runs no GPU picking and no provider request.
- On nonresident streams without a packed cache, or while it is still being prepared, picking returns `null` immediately. The raw WASM `begin_stream_pick_*` / `finish_stream_pick_*` keep only their signatures for compatibility, and the Promises they return reject immediately. Picking on resident charts remains supported. Highlighting an index that is already known reads only the needed original ranges.
- `export_png` redraws the source at the document size and the export scale. It does not stretch the screen texture and does not change the processing position or the accumulation image of the on-screen job.
- `export_svg` / `export_pdf` reject a chart shown from a stream; they export resident charts only.

### GPU caches and memory limits

- `inspect_column_admission(metadata)` reports the encoded size, the limits and the refusal reason for registering whole columns explicitly. It does not reserve space or guarantee the allocation of every derived resource, and it is not the criterion for running automatic streaming.
- `configure_auto_residency(memoryBudgetBytes, workingSetLimitBytes)` sets the total GPU budget and the per-chart GPU cache limit in bytes. It returns an error when the budget is smaller than the renderer's current usage. For supported precise-mode point, solid-line and error-bar charts, only the original rows the current view needs stay on the GPU during streaming. Completed packed pages move to the GPU; the CPU keeps only the current chunk and the page being written. The original rows of lines that join off-screen points but cross the view, and of error bars that reach the view, are included. A residency limit of 0 disables the packed cache. Connected columns are never put into `ColumnPool` automatically.
- When the display range narrows or the window size, aspect ratio or DPR changes, the view is redrawn from the packed original rows alone if the cache covers the area the new view needs. The cache covers line width, point size, error-bar caps and the AA footprint, plus an 8-pixel margin at the edges; this extra data counts toward the same packing limit. Coverage is checked as actual containment in data coordinates rather than by matching screen sizes, and when the cache does not reach a newly needed edge, the original ranges are requested again. Unsupported primitives, an exceeded packing limit, a short GPU budget and allocation failures keep the streaming path that draws every original primitive. Check the status with `state`, `needed_bytes`, `refusal_reason` and `picking_available` of `stream_status().view_residency`. `needed_bytes` is the minimum number of bytes needed so far while the cache is being prepared, and the actual packed size after completion. `job.cancel()` releases the job and its display resources.
- Dashed lines, histograms, heatmaps, sketch, constellation and supported Milkyway streams use a separate GPU cache that preserves source order and adjacency. It is built only when the total hi/lo encoded size of all referenced columns fits the per-chart residency limit and the device's single-buffer limit, and the total GPU budget has room for it. Chunks are copied into it on the GPU as they are uploaded, and no CPU copy of the source remains. After a size, DPR, axis-range, grid or heatmap colour-map change, the view is recomputed from the cached source, so it is never a blurred enlargement of an earlier image and keeps no trace of the previous settings. Ranges outside the cache, a new data revision and an exceeded budget use ordinary streaming. A large column is never cut so that part of it fits this cache. Keep the source provider for export and cache misses.
- The source-range cache reduces the cost of supplying data again. `view_residency` reports the packed view and whether immediate picking is available, so a chart with only this cache still shows `streamed`. Lowering the residency limit to 0 also releases the source-range cache. On a cache hit, native `auto_stream_chart_request_ranges()` can advance the GPU work of one chunk and return `Backpressure`. Keep calling it until completion as before, and supply source data only on `Ready`. For a read-only query, use `stream_status()`, which advances no work.
- The web column pool starts at 16 MiB and grows when upload space runs out. The default is `true`, and growth never exceeds the device's storage-binding limit or the total GPU budget. `set_pool_auto_growth(false)` keeps the current capacity. Allocation errors are told apart by the JS `Error.code` values `pool_space`, `budget_exceeded`, `device_limit` and `allocation_failed`. Where available, `requestedBytes`, `limitBytes`, `largestFreeBytes` and `totalFreeBytes` are also provided, as numbers within the range of integers JavaScript represents exactly and as decimal strings beyond it.
- `gpu_memory_status()` returns the live, retired and total requested allocations (`live/retired/total`); the pool capacity, occupancy, largest free region, backup and bytes pending release; the usage per resource; and each column's `resident` / `streamed` state. It does not measure the actual remaining VRAM. The precomputed textures of the Milkyway and constellation styles are excluded from the general totals; their size is considered separately only when checking whether a streaming export can run.
- `await release_unused_gpu_memory()`, when no stream is active, moves the remaining columns into a smaller pool on the GPU and waits for the work on the previous buffer to complete. At least 16 MiB is kept. The compaction also needs a temporary buffer, so budget, device-limit and allocation errors can occur. On failure the existing columns and bindings are kept.
- As an example of a host policy, the total budget can be `4_000_000_000` bytes with a per-view packing limit of `500_000_000` bytes. When the current view's cache exceeds the limit, streaming stays in use. When space or budget runs short while registering columns, remove unneeded columns, wait for cleanup to complete, and retry once. If it still fails, or a device-limit or actual allocation error occurs, switch to streaming. These numbers are an example, not renderer defaults.

The stream accumulation of precise styles stores data only. The grid is composited underneath at display time, so changing its visibility, colour, width or line style keeps the job ID and the data processing position. Streams without a resident cache also update the grid without replaying the source. Other styles that depend on the background redraw the data when the grid changes. With a source-range GPU cache, the host is not asked for the same chunks again. The separate compositing of precise styles does not resample a texture or apply AA again. UNORM colour compositing and MSAA rounding mean, however, that the colour channels are not guaranteed to be bit-identical to the resident path, which draws directly onto the background. Regression scenes limit the channel difference to at most 2 in 8-bit values, and scenes where translucent points, bar fills and outlines overlap are checked against at most 3.

`crates/web/tests/view-cache-resize-grid-probe.html` checks that window size and aspect-ratio changes request no source ranges, that the result matches pixel for pixel a display streamed from the start at the new size, and that grid changes apply without restarting the job.

### Support scope

- Nonresident heatmaps support drawing, selection highlights and export, but not immediate picking. Highlighting a known cell reads only the neighbouring axis values needed to compute its bounds. Auto-fit uses the existing GPU lattice computation and stores the result per revision.
- `streaming_capabilities()` returns whether the current configuration and series are supported, with the reason. `operations_require_completed_revision` tells whether queries and export need a completed job. Unsupported combinations are never ignored or switched to another style. Streaming does not support contours or Milkyway lines and their star chains. Polar charts, area fills and step lines, vector series, point colour and size maps, and overlay layers and subplots also need resident columns, so connecting a streamed source to them returns an error. Dashed lines in the precise, sketch and constellation styles keep the operation order of the existing path-length scan, so the dash pattern does not restart when chunks change.

The browser regression page is `crates/web/tests/streaming-contract-probe.html`. It checks 4.3 million points, Worker range supply, a resident chart shown at the same time, resizing, cancellation and range fitting, 1x and 2x export, and that picking on a stream without a packed cache returns at once and reads no source ranges.

Raw `pick_point` fulfils with a JSON string or `undefined`, and the web wrapper's `pick_point` with an object or `null`; errors arrive as Promise rejections. A submitted request holds its readback resources and the `Arc`-based `source_id` / `series_id` mapping current at submission. If the chart or the pool changes, or the renderer is released, while the request is pending, its target does not change.

<a id="등록해제-모델--메모리는-내부-자동-관리"></a>

## 7. 데이터 등록·제거와 메모리 관리

차트는 캔버스마다 인스턴스 하나를 두고 ID로 데이터를 등록·교체·제거한다. 풀의 할당, 재배치와 자원 해제는 렌더러가 처리한다.

- 컬럼 등록·일괄 등록·교체 규칙은 [3.3 데이터 등록·교체와 GPU 저장 형식](#33-데이터-입력--명시적-registerupdate와-f32-물리-lane)을 따른다.
- 업로드할 때 자동 범위 맞춤용 최소·최대·최소 양수 값을 저장한다. 점선 간격에 필요한 누적 경로 길이 같은 점별 정보는 GPU 스캔(`line_arc.wgsl`)으로 계산한다.
- `remove_column(id)`는 해당 컬럼을 참조하는 시리즈도 제거한다. 자동 범례에서는 대응하는 행을 지운다. `set_config`로 직접 편집한 범례는 사용자 텍스트를 보존하고 남은 시리즈의 인식 가능한 기호만 갱신한다.
- 제거·교체로 생긴 빈 공간은 렌더러가 정리 작업으로 등록하고 다음 `frame()` 시작에 GPU 내부 복사로 모아 정리한다. 연속 교체 중 할당 공간이 부족하면 `OnAllocFailure` 정책으로 재배치를 시도한다.
- `add_line_series`는 기존 ID이면 색을 유지하며 교체하고, 새 ID이면 색상 순서의 다음 색을 사용한다. 기존 ID에 빈 라벨을 전달해도 기존 범례 텍스트는 지우지 않는다. 비어 있지 않으면 해당 행의 텍스트만 바꾼다.
- `free()`로 인스턴스를 해제하면 풀 버퍼·파이프라인·텍스처·surface의 참조를 정리한다. GC의 FinalizationRegistry도 정리를 지원하지만 실행 시점이 정해져 있지 않으므로 SPA에서 컴포넌트를 제거할 때는 `free()`를 직접 호출한다.

<a id="categorical-documents"></a>

### Categorical bar documents

`FiggyChart.set_categorical_document(json)` opens a categorical bar
document (`CategoricalChart` JSON, which `FiggyBoundedChart` took as the
`"categorical"` family before this release) in the Cartesian wrapper. The
document is validated and read by `Config::from_categorical`: its
categories become the categorical axis of the bar direction, each of its
series a `CategoryBars` series, and its values two kinds of registered
columns, `categorical.positions` and `categorical.values.<series id>`, the
value columns registered as one batch. The value axis is fitted and rounded to nice ticks; the
chart keeps its chart area. Opening another document replaces the series
and the columns of the previous one; every other column stays. From there
on the chart is edited through the Cartesian API (`get_config` /
`set_config`, `get_series` / `set_series`, the column registry), and bars
are picked with `pick_data` like histogram bins. The bars' value labels
(`labels.text`, written by the reader) are drawn at the bar ends; a label
is picked by `pick_data` (`kind: "value_label"`) and `pick_point`, both with
its `label_id`, and dragged with `press_value_label(label_id)`, `on_move`
and `on_release`, which write the series' `labels.offsets`.

```js
chart.set_categorical_document(JSON.stringify(categoricalDoc));
await chart.first_frame_ready();
```

<a id="box-plot-documents"></a>

### Box plot documents

`FiggyChart.set_boxplot_document(json)` opens a box plot document
(`BoxPlotChart` JSON, which `FiggyBoundedChart` took as the `"boxplot"`
family before this release) in the Cartesian wrapper. The document is
validated and read by `Config::from_boxplot`: its categories become the
categorical axis of the box direction, each of its series a `BoxPlot`
series, and its summaries three kinds of registered sources: the
positions column `boxplot.positions`, one record of box statistics per
series (`boxplot.record.<series id>`, as `register_record` registers one)
and, for a series with outliers, one ragged outlier column
(`boxplot.outliers.<series id>`, as `register_ragged`). The summaries are
uploaded through borrowed adapters, without a copy. Without a document
`value_range` the value axis is fitted to the records' and outliers'
statistics and rounded to nice ticks; the chart keeps its chart area.
Opening another document replaces the series, columns and records of the
previous one; every other column stays. From there on the chart is edited
through the Cartesian API. Every part of a box (body, whiskers, caps,
median, mean, outliers) is picked with `pick_data` as the box's
`HistogramBin`; `selection_details()` reports the selected box's
`category_id`, `category_label`, `index`, `direction` and its statistics
(`median`, `q1`, `q3`, `whisker_low`, `whisker_high`, `mean`,
`sample_count`, `ci_low`, `ci_high`; a missing one is `null`), read back
from the record. `step_selection(dx, dy)` walks the categories and the
series of the box group (boxes, not outliers). The value labels the
document configures are written into `labels.text` and drawn past the top
of each box; they pick and drag as the bars' do.

```js
chart.set_boxplot_document(JSON.stringify(boxPlotDoc));
await chart.first_frame_ready();
```

<a id="radar-documents"></a>

### Radar documents

`FiggyChart.set_radar_document(json)` opens a radar document (`RadarChart`
JSON, which `FiggyBoundedChart` took as the `"radar"` family before this
release) in the Cartesian wrapper as a polar chart. The document is
validated and read by `Config::from_radar`: its categories become the
categorical angle axis (category `i` of `n` at `i / n` of the turn from the
document's start angle, in its direction), each of its series a `Radar`
series, and its values two kinds of registered columns, `radar.positions`
and `radar.values.<series id>`, uploaded through borrowed adapters without a
copy. The document's grid shape becomes `PolarOptions.grid_shape`. Without
a document `value_range` the radius axis is fitted to the value columns'
statistics, from zero when no value is negative, and rounded to nice
ticks; a document `ring_spacing` is kept either way. The chart keeps its
chart area. Opening another document replaces the series, columns and
config of the previous one; every other column stays. From there on the
chart is edited through the Cartesian API. A vertex is picked with
`pick_data` as a point of its series (the outline, its closing segment and
the fill pick the series too), `selection_details()` adds the vertex's
`category_id` and `category_label`, and `step_selection(dx, dy)` steps the
vertices of the series and across the radar series. The value labels the
document configures are written into `labels.text` and drawn outside each
vertex along its spoke; they pick and drag as the bars' do.

```js
chart.set_radar_document(JSON.stringify(radarDoc));
await chart.first_frame_ready();
```

<a id="bounded-chart-host"></a>

## 8. Radial chart host

`FiggyBoundedChart.create(canvas, family, documentJson, width, height)` exposes
`radial` documents, the DataLess radial chart, to browser hosts. The class
keeps its name and `family` stays `"radial"`. For this release the
`"categorical"`, `"boxplot"` and `"radar"` families are rejected with an error
naming the `FiggyChart` method that opens those documents
(`set_categorical_document`, `set_boxplot_document`, `set_radar_document`; see
[Radar documents](#radar-documents) and the sections above it). It
uses the same native renderer, decoration hit map, slice picking,
and drag/resize policies. The host owns pointer events, animation scheduling,
and synchronization.
`width` and `height` are the document's logical size: at least 160 by 160.
The chart is drawn
at the scale that fits this size into the canvas's pixel size (the smaller of
the two ratios, clamped to 0.5 to 4). After the canvas size changes, pass its
new pixel size to `resize(width, height)`.
Creation asynchronously precompiles the radial rendering pipeline
and prepares its document. It does not submit the first visible frame: call
`await view.first_frame_ready()` before reporting Ready. That method submits a
frame and awaits the actual GPU completion Promise; rejection/device loss is
returned to the host. `frame()` alone only submits work.

`set_document(documentJson, width, height)` validates and prepares the new
document before replacing the whole document, and keeps the previous one on
failure. A `radial` document's slices draw their `text`: `create` and
`set_document` fill a missing one from the slice's label format (as
`RadialChart::normalize_labels` does) and keep one that is set, so after
editing a value or a format remove the affected `text` fields (or write the
new strings) before sending the document back. Omitted fields are not kept: they take their defaults or fail to
parse, so read `get_document()`, change the fields you need and send the
complete document back.

The interactive editing and export methods are:

- `decorations()` returns the measured decorations as JSON
  `[{ id, rect, handles }]`, with `rect` and each handle as
  `[x, y, width, height]`.
- `hit_decoration(x, y, editArea)` resolves to the id of the topmost
  decoration under the point, or `undefined`. With `editArea` false, a point
  over data inside an unselected data area resolves to `undefined`, so the
  click can go to `pick`. It is asynchronous because a radial chart answers
  "data here" with its GPU pick.
- `select_decoration(id | null)` selects the decoration with that id, or none,
  and clears the data selection; an unknown id throws.
- `resize_handle(id, x, y)` returns the compass name of that decoration's
  resize handle under the point, such as `"SE"`, or `undefined`.
- `drag(id, dx, dy, handle)` moves the decoration, or resizes it when `handle`
  is a compass name such as `"SE"` (`null` for a move), and returns whether it
  moved.
- `pick(x, y, toggle)` selects the mark under the point, clears the decoration
  selection and resolves to the data selection as JSON (`"null"` for none).
  With `toggle` true, picking the selected mark again clears it.
- `hover(x, y, dt)` resolves to whether another frame is needed; `dt` is the
  time since the previous call in seconds, capped at 0.1. Pass `null` for both
  coordinates when the pointer leaves.
- `pick`, `hover` and `hit_decoration` return Promises, like
  `FiggyChart.pick_point`: a radial chart picks its slices on the GPU with the
  draw's own sector ray-march and reads the answer back. Await each before the
  next call on the object.
- `step_selection(backwards)` takes one boolean, unlike the Cartesian two-axis
  navigation method.
- `get_document()`, `clear_selection()`, `resize(width, height)`, `frame()`,
  async `export_png(scale)` (scale 0.5 to 4), and async `export_svg(scale)`
  (a `String`) and `export_pdf(scale)` (a `Uint8Array`) complete the set. The
  SVG and PDF hold the slices and background as one image and the title,
  labels, leader lines and annotations as vector content, with every inside
  label above every slice and no selection box.

Coordinates and drag deltas are document logical pixels; canvas resize does not
change export size. Keep the object alive until all async calls finish;
`free()` releases its GPU resources.

```js
import init, { FiggyBoundedChart } from "./pkg/figgy.js"; // figgy-chart.js does not export it
await init(); // load the WASM module before the first create
// radialDoc is a validated RadialChart value, e.g. serialized by its model.
const view = await FiggyBoundedChart.create(canvas, "radial", JSON.stringify(radialDoc), 960, 600);
// Categorical, box plot and radar documents open in FiggyChart
// (set_categorical_document, set_boxplot_document, set_radar_document).
await view.first_frame_ready(); // only now report Ready and enable interactions
const point = [320, 240]; // logical document coordinates
const selected = JSON.parse(await view.pick(...point, true)); // click toggles selection
view.select_decoration("chart_title");
view.drag("chart_title", 12, 6, null); // changes only this decoration's offset
view.frame();
const png = await view.export_png(1);
view.free();
```

### Annotation layer

Both classes expose the annotation layer with the same method set:
`annotations()` (drawn items as JSON: `index`, `id`, `element`, `kind`,
selection `rect` and `handles`, `selected`), `hit_annotation(x, y)` (the
topmost item index or `undefined`), `select_annotation(index | null)` and
`selected_annotation()`, `annotation_handle(index, x, y)` (the live handle's
compass name, e.g. `"W"`), `drag_annotation(index, dx, dy, handle | null)`
(returns `false` for a rejected move), `add_annotation(json)` (returns the new
index), `set_annotation(index, json)`, `remove_annotation(index)`,
`set_annotations_visible(bool)`, `pointer_to_anchor(x, y, "Data" | "Ratio")`
(the `Anchor` at a point as a JSON string, or the JSON string `"null"` when
the point has no such position, for example a `Data` anchor on a chart without
value axes; `JSON.parse` turns it into `null`) and `annotation_warnings()`
(`[{index, reason}]` for items the raster skipped). Item JSON is the
`Annotation` shape of the config document (see the
[JSON schema](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)).
Coordinates are the same as the class's other hit methods: document logical
pixels on `FiggyBoundedChart`, physical canvas pixels on `FiggyChart`. On
`FiggyBoundedChart` the selection is `decorations.selected`; on `FiggyChart`
it is host state like the decoration selection, and selecting an annotation
clears the decoration selection and vice versa.

On `FiggyChart` the methods that read drawn items (`annotations`,
`hit_annotation`, `annotation_handle`, `drag_annotation`,
`pointer_to_anchor`, `annotation_warnings` and
`decoration_bounds("annotation_<n>")`) use the annotation map of the last
drawn frame. After `add_annotation`, `set_annotation`, `remove_annotation` or
a drag they report the new state once the next `frame()` has drawn it: until
then a new item is not listed, `hit_annotation` does not find it and
`drag_annotation` on its index throws. `<figgy-chart>` draws on its
animation-frame loop, so read them after the next animation frame.
`FiggyBoundedChart` prepares the frame and its annotation map inside each
editing call, so its methods report the new state as soon as the call returns.

The radial chart has no value axes: a `Span`, a reference line and a
`Data` anchor are skipped and listed by `annotation_warnings()`, and the frame
paints its opaque background over a white clear with the annotations above
it.

The pointer methods follow the active layer. `FiggyBoundedChart` has
`set_active_layer(json)` / `active_layer()` (`"Base"` or `"Annotations"`; an
`{ "Overlay": n }`, `{ "Subplot": i }` or `{ "SubplotOverlay": [i, j] }` is
stored as `"Base"`, and the stored layer is returned as
JSON). While `"Annotations"` is active and the layer is shown,
`hit_decoration` answers drawn items as `"annotation_<n>"` above every
decoration, and `select_decoration`, `resize_handle` and `drag` accept that id;
on `"Base"` the items are not part of the route and clicks pass through them.
`pick` and `hover` keep addressing slices on
every layer. A
layer change clears `decorations.selected` (an annotation included) and keeps
the data selection; the frame and its label raster are reused. On `FiggyChart`
the same route drives `hit_test`, `on_press` / `on_move` and
`decoration_bounds`, as listed in the method table above. The index-based
annotation methods keep working on every layer.

### Cartesian selection helpers (`FiggyChart`)

`FiggyChart.decoration_bounds(id)` returns measured bounds (`rect`)
and resize handles (`handles`) in physical canvas pixels. For an element whose
selection outlines its drawn parts (polar angle and radius tick labels, the
polar ray, annotations), it also returns `outline`: a list of polygons, one per
drawn part, each a list of `[x, y]` corners in physical canvas pixels. They are
exactly the shapes the canvas strokes around the selection: a text block or
rectangle as its box padded by 2 logical px, a stroke as the thin oriented box
around it, and an arrow head as its triangle grown outward. `step_selection(dx, dy)` navigates
resident point, histogram or matrix selections using source indices, without
copying arrays. For a point, `dx` moves the source index and `dy` switches
between the series that draw points or lines; a selected point-vector arrow
steps along its series' points like a scatter point. Histogram bin navigation
follows the category direction: `dx` for vertical bars and `dy` for horizontal
bars; the other delta switches between series of the same orientation. A
category bar steps along its categories by the category axis' delta (`dx`
for vertical bars, `dy` for horizontal ones) and across the series of its
bar group by the other. A
matrix cell, a grid-vector cell included, steps by `dx` along X and `dy` along
Y. Indices clamp to the selected series. The rule is the model's
`Config::step_selection(series, dx, dy, column_len)`, which native hosts call
with their own column lengths.
`await chart.selection_details()` returns a JSON string with selected resident
GPU values (or `"null"` for no supported selection). Histogram fields are
`identity`, `series_id`, `index`, `bin_start`, `bin_end`, `value`, and
`orientation`; a category bar reports `identity`, `series_id`, `index`,
`category_id`, `category_label`, `value`, `direction` and `mode`; `identity`
uses a flat `kind` tag. Points and matrix cells are
also supported; a colour- or size-mapped point adds `color_value` / `size_value`,
and a vector selection adds `u` / `v` (plus `color_value` with a colour map).
A point of a ternary series adds `c`, its third component as uploaded.
`<figgy-chart>` has no forwarding methods for these or for the annotation
methods; call them through [`element.kernel`](#figgy-chart-kernel).
See the [selection JSON contract](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md#resident-selection-details).
Hold the host's async gate during selected-value reads as well as exports.
