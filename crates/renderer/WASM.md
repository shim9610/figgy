<a id="webassembly-빌드와-웹-io-가이드"></a>

# WebAssembly 빌드와 브라우저 사용법

이 문서는 소스 릴리스 `figgy 0.11.1` / `figgy-renderer 0.13.1` / `figgy-model 0.8.0`를 기준으로 한다. 아래 스트리밍 API를 사용하려면 해당 버전의 소스에서 브라우저 패키지를 빌드한다.

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

CPU 렌더링에는 순수 Rust로 구현된 tiny-skia·fontdb·swash를 사용한다. 기존 skia-safe의 `wasm32-unknown-emscripten` 대상은 wasm-bindgen에서 사용하는 `wasm32-unknown-unknown`과 함께 쓸 수 없어 교체했다.

Liberation Sans 네 가지 글꼴을 내장하므로 웹에서도 이 폰트의 문자를 그릴 수 있다. 시스템 폰트 검색은 네이티브에서만 지원한다. 웹에서 다른 폰트를 쓰려면 `register_font(Uint8Array)`로 TTF/OTF 파일을 등록한다. 폰트는 등록된 폰트, 시스템 폰트, 내장 대체 폰트 순으로 찾는다.

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
| — | `Renderer::wait_idle` | 웹에서는 별도 작업 없음(브라우저가 장치 완료를 처리) |
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
| 저수준 `FiggyChart` | `create` / `create_with_progress`는 같은 GPUDevice에서 모든 렌더링용 WGSL 진입점을 `createRenderPipelineAsync`로 미리 컴파일하고 임시 JS 파이프라인을 해제한다. 이후 빈 차트의 첫 프레임 완료를 기다린다. 선택적인 렌더링·스타일 자원과 경로 길이·범위 계산·피킹·등고선 컴퓨트 캐시는 처음 필요할 때 만든다. `prewarm_all_with_progress(callback)` / `prewarm_all()`로 이 캐시들을 미리 생성할 수 있다. `warm_up()`은 첫 프레임 대기용 호환 메서드이며 전체 사전 준비를 하지 않는다. 생성 시 피킹은 켜지 않으며, `prewarm_gpu_picking()`과 `pick_point` / `pick_data`가 같은 준비 경로와 저장된 활성화 오류를 사용한다. |
| 래퍼의 준비 완료 | `web.create / first frame / finished` 진행 알림과 `figgy-ready`를 보낸 뒤 백그라운드에서 피킹을 준비한다. 실패하면 복구 가능한 `figgy-error`로 알린다. 이미 완료된 `ready`와 렌더링 루프는 유지한다. |
| 래퍼의 작업 중 상태 | 연결 세대와 커널을 함께 확인하는 작업 토큰으로 초기화와 상태를 바꾸는 비동기 WASM 호출을 직렬화한다. 두 전체 사전 준비 메서드도 이 경로를 따른다. 작업 중에는 프레임·입력·동기 대리 호출이 WASM에 접근하지 않는다. 마지막 크기 변경과 포인터 해제만 보관했다가 작업 종료 후 적용한다. |
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
│ Blob 다운로드 ◀───────┼─ async ─────│ export_png_bytes_async       │
└──────────────────────┘             └─────────────────────────────┘
```

<a id="31-그리기-표면--데스크톱과-같은-두-경로"></a>

### 3.1 캔버스와 장치 초기화

- **웹 래퍼 사용(권장)**: `<figgy-chart>`를 배치하면 래퍼가 shadow DOM 안에 캔버스를 만들고 `FiggyChart.create(canvas)`를 비동기로 호출한다. `await element.ready` 또는 `figgy-ready` 이후 `register_column_f32`, `update_register_column_f32`, `set_series`, `export_png` 등의 메서드를 사용할 수 있다.
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

준비 중에는 wasm-bindgen이 객체의 변경 가능한 참조를 사용한다. 호스트는 해당 차트 호출을 대기시켜야 하지만 다른 앱 작업까지 막을 필요는 없다. 웹 래퍼의 두 메서드는 연결 세대를 확인하는 기존 작업 직렬화 장치를 사용한다. 같은 출력 형식·샘플 수에서 사전 준비가 끝난 상주 경로가 다시 파이프라인을 만든다면 회귀 오류다. 비상주 스트리밍의 행렬·타일 전용 파이프라인은 범위에 포함되지 않으며 스트리밍 준비 시 생성한다. `warm_up()`은 `first_frame_ready()`와 같은 작업을 하는 호환 메서드다.

초기화 시간은 호스트에서 `performance.now()` 같은 단조 시계로 측정한다. 별도 WASM 함수나 커널 내부 타임스탬프는 필요하지 않다. `FiggyChart.create_with_progress`의 시작·완료 시각과 진행 콜백 수신 시각을 기록하면 된다. 일반 생성은 창·렌더러 준비, 각 비동기 렌더링 진입점, `web.create / chart resources`, `web.create / first frame` 순서로 started/finished 알림을 보낸다. 파이프라인별 첫 제출을 비교하려면 표준 등록·설정·프레임 API로 선·점·오차 막대 작업을 각각 구성하고 같은 외부 시계로 측정한다.

피킹 사용 여부를 비교할 때는 비활성 조건에서 `prewarm_gpu_picking()`을 호출하지 않는다. 활성 조건에서는 `web.create / first frame / finished` 콜백 이후 이 메서드를 직접 호출하고 소요 시간을 따로 기록한다. 피킹 컴파일 시간을 차트 생성 시간에 합치거나, 첫 선택에서 자동으로 준비된 시간을 명시적인 사전 준비 시간으로 기록하지 않는다.

`FiggyChart.create`는 피킹을 켜지 않는다. 첫 선택 전에 준비하려면 `await chart.prewarm_gpu_picking()`을 호출한다. 이 메서드와 `pick_point` / `pick_data`는 렌더러의 `enable_gpu_picking_async`를 거친 뒤 현재 차트의 피킹 캐시를 준비한다. 파이프라인·캐시·리비전은 렌더러가 관리하므로 반복 호출과 재시도도 저장된 활성화 오류를 포함한 같은 상태를 사용한다.

`Renderer`는 차트별 설정과 시리즈 순서, `ColumnPool`, 피킹 파이프라인, 현재 차트용 피킹 캐시 하나, 대기 중인 정리 작업을 관리한다. 웹 커널은 UI용 파생 메타데이터와 Promise 변환을 담당하며 피킹 엔진·변경 플래그·풀 정리 상태를 중복 관리하지 않는다.

Renderer 0.9부터 `GpuPickEngine`은 공개 API에서 제외됐다. 네이티브·임베드 호스트는 `enable_gpu_picking()`, 필요에 따른 `prepare_gpu_picking_for_chart(chart_id)`, `pick_chart(chart_id, GpuPickRequest)` 순서로 사용한다. `WindowedRenderer::pick_chart_at`은 현재 surface에 맞는 패널 위치와 배율도 계산한다. 호스트가 축 변환이나 데이터 영역 자르기 계산을 중복할 필요는 없다.

점·막대·셀·등고선을 구분하는 선택에는 `pick_chart_data` / `WindowedRenderer::pick_chart_data_at`을 사용한다. 결과에는 종류별 식별자와 `distance_px`가 들어간다. 막대·행렬 선택용 컴퓨트 셰이더는 그리기와 같은 좌표 변환·풀·스타일·격자·레벨 표·도형 계산 함수를 사용한다. CPU는 끝점의 f64 좌표, 막대 사각형, 셀 경계, 등고선 선분을 복원해 보관하지 않는다. 결과를 `set_picked_data`로 전달하면 그리기에 사용한 같은 바인드 그룹으로 선택 표시를 그리므로 축이나 데이터가 바뀌어도 위치가 맞는다.

<a id="32-렌더-루프--requestanimationframe--renderer-stamp"></a>

### 3.2 렌더링 루프와 프레임 갱신 판단

`<figgy-chart>`는 `requestAnimationFrame` 콜백에서 데스크톱 예제와 같은 갱신 판단을 수행한다. 저수준 커널을 직접 사용한다면 이 루프도 호스트가 구현해야 한다. 다음은 실제 `frame()`의 상태 변화를 요약한 의사 코드다.

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
    renderer.has_pending_maintenance(),
) {
    Clean => return Ok(()),
    MaintenanceOnly => {
        renderer.process_pending_maintenance()?; // surface acquire/draw 없음
        return Ok(());
    }
    Draw { refresh_raster } => {
        ensure_internal_render_columns()?;
        renderer.process_pending_maintenance()?;
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

`WindowedRenderer::draw`는 `Renderer::prepare`와 `Renderer::paint_prepared`를 한 번에 실행한다. 준비 단계는 `&mut self`로 파이프라인 준비, 좌표 변환 유니폼 기록, 경로 길이 컴퓨트 실행을 수행한다. 기록 단계는 `&self`로 그리기 명령만 만든다. WASM 래퍼처럼 렌더러를 단독으로 관리하는 호스트는 이 통합 호출을 사용하고, 공유 참조만 받는 paint 콜백에서는 두 단계를 나눠 호출한다. 자세한 내용은 README의 통합 예제를 참고한다.

변경이 없는 rAF에서도 다음 콜백 예약, DPR 비교와 WASM 상태 확인은 수행한다. GPU 컬럼 준비, surface 획득, 그리기·제출·화면 표시는 생략한다. 갱신이나 그리기가 실패하면 마지막 표시 기록과 호스트 플래그를 유지해 다음 rAF에서 다시 시도한다. 이 최적화는 이전 화면이 그대로 유효할 때만 적용하며 데이터 샘플링·LOD·데시메이션이나 시간에 따른 프레임 생략은 하지 않는다.

`interrupt_render()`가 스트림 취소를 예약하면 다음 프레임도 예약된다. 프레임 시작의 `process_pending_maintenance()`는 취소를 먼저 처리하고, 그 뒤에 스트림 화면과 상주 차트 중 어느 경로로 그릴지 결정한다. 풀 재배치가 없어도 취소는 처리된다. 같은 커널에 다른 차트의 설정과 시리즈를 복원하면 복귀 첫 프레임부터 창 크기에 맞는 좌표와 잘라내기 영역을 사용하며, 저장된 축 범위는 유지한다. 객체 선택이나 축 조작으로 다시 그릴 필요가 없다.

`crates/web/tests/stream-resident-return-probe.html`은 **상주 차트 확대 → 스트림 → 상주 차트 복귀**의 첫 화면을 실제 캔버스 픽셀로 비교한다. 기본 창은 1200×800이며 `?width=600&height=900&partial=1`로 세로 창에서 스트림 도중 복귀를 검사할 수 있다. `cancelOnly=1`을 추가하면 설정 변경 없이 취소만 해도 다음 프레임에 반영되는지 함께 검사한다. PNG 내보내기 결과로 화면 검사를 대신하지 않는다.

컬럼 교체·제거·재배치는 임시 풀, 영향을 받는 차트 상태·리비전, 새 피킹 캐시를 먼저 준비한다. 동기 준비 중 오류가 나면 기존 상태를 유지한다. 성공하면 풀·차트·피킹·정리 상태를 추가 할당 없이 반영한다. `remove_column`은 해당 컬럼을 참조하는 시리즈만 함께 제거하고 범례 문서는 바꾸지 않는다. 범례 설정도 함께 바꿔야 한다면 `remove_column_with_chart_config`로 한 번에 반영한다.

`FiggyChart::load_demo()`도 데모 상태 전체를 한 번에 교체한다. 컬럼 4개, 최종 설정과 시리즈 순서, 피킹 상태, 웹의 컬럼 리비전·스타일·라벨·색상 메타데이터, 유효한 범위 캐시를 함께 반영한다. 확정 전에 동기 오류가 나면 모두 이전 상태를 유지한다. 트랜잭션 중에는 범위 계산 명령을 제출하지 않으며, 변경된 컬럼을 참조하던 캐시는 제거한 뒤 필요할 때 다시 만든다. 이 작업에는 풀 용량과 같은 임시 GPU 버퍼 하나와 스테이징 버퍼 4개가 추가로 필요하다. 재배치 백업이 남아 있으면 원본 풀·백업·임시 풀이 잠시 공존한다.

<a id="33-데이터-입력--명시적-registerupdate와-f32-물리-lane"></a>

### 3.3 데이터 입력 — 신규 등록, 교체와 GPU 저장 형식

GPU 풀은 논리값 하나를 **항상 f32 두 개**로 저장한다. 일반 컬럼은 `(value as f32, 0)`, `Float64Array` / `HiLoColumnSource`는 `(hi: f32, lo: f32)`를 기록한다. 셰이더의 f64 연산 대신 두 f32의 합을 이용해 큰 절대값 안의 작은 차이를 보존한다. 이 저장 형식은 네이티브와 WASM에서 같다. 다음 예는 매핑된 스테이징 버퍼를 준비한 뒤 실행하는 내부 업로드 코드다.

```rust
// scalar: logical value → `(value as f32, 0)`
let mut view = staging.slice(..).get_mapped_range_mut();
let writer = ColumnPairWriter::new(view.slice(..)); // renderer pool 내부, crate-private
let stats = source.write_f32_pair_le_into_with_stats(writer);
drop(view);
staging.unmap();
enc.copy_buffer_to_buffer(&staging, 0, &pool, offset, None);  // 이후는 GPU 내부 복사

// hi/lo: logical f64 value → two f32 lanes in a separately mapped staging buffer
let mut view = staging.slice(..).get_mapped_range_mut();
let writer = ColumnPairWriter::new(view.slice(..)); // renderer pool 내부, crate-private
let stats = source.write_f32_pair_le_into_with_stats(writer);
drop(view);
staging.unmap();
```

네이티브에서는 원본 참조를 빌려 변환 결과를 매핑된 업로드 버퍼에 직접 쓴다. 데이터 소스는 값을 기록하는 같은 반복문에서 `ColumnUploadStats`도 계산하므로 렌더러가 쓰기 전용 영역을 다시 읽지 않는다. 최소 양수 값은 일반 컬럼의 실제 `value as f32`, hi/lo 컬럼의 `hi as f64 + lo as f64`를 기준으로 하며 유한한 양수만 포함한다.

사용자 정의 `ColumnSource` / `HiLoColumnSource`도 값 기록과 통계 계산을 함께 하는 메서드를 구현해야 한다. 구현이 빠지면 컴파일 오류가 난다. 매핑된 바이트를 다시 읽거나 부정확한 대체 경로로 처리하지 않는다.

WASM에서는 다음과 같은 메모리 경계 복사가 추가된다.

1. JavaScript에서 받은 데이터는 JS 힙에서 WASM 선형 메모리로 한 번 복사한다. WASM 안에서 생성하거나 직접 가져온 데이터에는 이 복사가 없다.
2. wgpu 30.0.1의 웹 백엔드는 쓰기 뷰를 만들 때 브라우저의 매핑 영역을 임시 WASM `Vec`로 복사하고, 뷰를 해제할 때 변경 내용을 되돌려 복사한다. 이는 wgpu가 JS·WASM 경계를 처리하는 방식이며 GPU 결과 읽기와는 다르다. 네이티브의 중간 변환 버퍼 없는 업로드에 이 비용을 포함해 설명하지 않는다.

입력 배열은 데이터 정밀도에 맞춰 고른다.

- **`Float32Array`**: 일반 좌표에 권장한다. JS·WASM 경계에서 원소당 4바이트를 전달하며, 빌린 데이터에서 `(value, 0)` 기록과 통계 계산을 한 번에 수행한다.
- **`Float64Array`**: 큰 절대 좌표에 사용한다. 경계 전달량과 GPU 저장량은 원소당 8바이트다. 최소·최대 메타데이터뿐 아니라 GPU 좌표 계산도 hi/lo 쌍을 사용하므로 타임스탬프 같은 큰 값에서 f32 하나로 표현할 수 없는 작은 차이를 보존한다.

인자 변환 비용을 줄이려면 WASM이 버퍼를 할당해 포인터와 길이를 제공하고, JS가 `new Float32Array(memory.buffer, ptr, len).set(src)`로 채우도록 구성할 수 있다. 이 방식도 경계 복사는 한 번 필요하며 wasm-bindgen의 인자 변환 과정만 줄인다.

공개 API는 등록과 교체를 구분한다:

```js
chart.register_column_f32("x", xs);          // 새 id만; 기존 id면 오류
chart.update_register_column_f32("x", next); // 기존 id만; 없으면 오류

chart.register_column_f64("time", times);          // Float64Array → hi/lo
chart.update_register_column_f64("time", nextTimes);
```

행렬 데이터는 컬럼 수가 많을 수 있으므로 연속된 배열 하나로 묶어 등록할 수 있다.

```js
// z 는 ids.length 개 컬럼 × valuesPerColumn 개 값, id 순서로 이어붙인 하나의 배열
const ids = Array.from({ length: 5000 }, (_, c) => `z${c}`);
chart.register_columns_f32(ids, z, 5000);   // 업로드 1회
chart.register_columns_f64(ids, z64, 5000); // f64 는 hi/lo 분할 유지
```

일괄 등록은 컬럼별 배열 목록 대신 연속된 배열 하나를 받는다. 직사각형 행렬의 메모리 배치를 그대로 사용하면 JS에서 컬럼마다 순회하고 WASM 경계를 반복해서 넘는 비용을 줄일 수 있다. 업로드할 때 각 컬럼은 전달된 버퍼의 일부를 빌려 스테이징 버퍼에 기록하므로 컬럼별 복사본을 만들지 않는다. JS·WASM 경계 복사는 앞서 설명한 규칙을 따른다.

- 새 ID만 등록할 수 있다. 배치 내 중복 ID, 이미 등록된 ID, `data.length !== ids.length × valuesPerColumn`이면 전체를 거부한다. 실패한 배치는 아무 ID도 등록하거나 업로드하지 않는다.
- 컬럼 길이가 서로 다른 데이터는 개별 `register_column_*` 호출로 등록한다.
- 리비전은 배치 단위가 아니라 컬럼마다 증가한다.
- `register_columns_f64`도 `(hi, lo)` 정밀도를 유지하며 f32 하나로 축소하지 않는다.

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

웹 래퍼는 이벤트마다 캔버스의 CSS 크기와 실제 픽셀 크기의 비율로 포인터 좌표를 계산한다. `FiggyChart`는 저장된 `chart_area`의 가로세로 비율을 유지해 현재 surface에 맞추고 남는 공간에 여백을 둔다. 드래그·크기 조절의 이동량은 내부에서 문서 좌표로 환산한다. 브라우저 창 크기를 바꾸는 것은 미리보기 배율만 바꾸며 출력 문서나 폰트 크기는 바꾸지 않는다.

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

### 3.6 PNG 내보내기 — 비동기로 `Uint8Array` 반환

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

동기 출력 함수 `export_panel_png_bytes`는 웹 빌드에서 제외된다. 웹에서는 비동기 버전을 사용해야 한다.

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

`Config`와 `Vec<SeriesConfig>`는 GPU 핸들이 없는 데이터 구조다. `model`의 `serde` 기능을 켜면 전체를 JSON으로 직렬화할 수 있다. 이 기능은 기본적으로 꺼져 있어 불필요한 의존성을 추가하지 않는다. 웹 래퍼의 `get_config` / `set_config` / `get_series` / `set_series`로 설정을 읽고 수정할 수 있다.

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

<!-- contour-contract: scope=wasm max-levels=1024 -->
`set_series(json)`에서 `Contour`와 `HeatmapContour`의 `contour.levels`는 `0..=1024`개를 허용한다. 1025개 이상이면 JavaScript 예외를 반환하고 이전 설정·시리즈·GPU 스타일을 유지한다. 다음 프레임도 이전 상태로 그린다. WASM에만 더 작은 상한을 두거나 배열을 잘라서 처리하지 않는다.
등고선 라벨도 자동·직접 지정 모두 1024개까지 지원한다. `spacing_px`는 라벨을 숨겼거나 앵커를 직접 지정했더라도 유한한 양수여야 한다. 자동 배치에서는 허용 범위로 제한한 프레임·출력 배율과 간격의 곱도 검사한다. 이 곱이 넘치거나 라벨 아틀라스가 어댑터의 텍스처 크기 한도를 넘으면 GPU 상태를 바꾸기 전에 실패해 이전 차트와 자원을 유지한다.

`set_config`는 JSON을 검증한 뒤 `set_chart_config(chart_id, config)`로 렌더러의 설정을 교체한다. desired/config/raster 리비전이 갱신되므로 다음 `frame()`은 `ChartRenderStamp`를 비교해 다시 그리기와 축 이미지 갱신을 수행한다.
축 스케일을 바꾸면 `major_spacing`의 단위도 바뀐다. 선형축에서는 데이터 단위이고 로그축에서는 10배 간격을 뜻하는 decade 단위다. `set_x_range` 같은 편의 함수는 이를 맞춰 주지만 JSON을 직접 편집할 때는 함께 수정해야 한다. `AxisOptions.inverted`도 JSON 필드로 지정한다. 축·데이터·피킹이 같은 방향 반전을 적용한다.

전체 필드와 열거형 값, serde 표현 규칙, 함께 수정해야 하는 옵션은 [SCHEMA.md](../web/SCHEMA.md)를 참고한다. 이 문서의 JSON 예시는 Rust 소스에서 생성하며 저장소 문서 정합성 검사에서 일치 여부를 확인한다.

<a id="39-async-메서드와-객체-잠금-필독"></a>

### 3.9 비동기 호출 중 객체 접근 제한

wasm-bindgen의 비동기 메서드가 실행 중일 때 같은 객체의 다른 메서드를 호출하면 `"recursive use of an object"` 예외가 발생한다. 웹 래퍼는 이를 내부에서 방지한다. 저수준 커널을 직접 사용한다면 다음 규칙을 지켜야 한다.

- rAF 루프는 WASM을 호출하기 전에 다음 `requestAnimationFrame(tick)`을 예약한다. 예외가 발생해도 루프가 끊어지지 않게 하기 위해서다.
- 생성·연결, 두 전체 사전 준비 메서드, `prewarm_gpu_picking`, 출력, `first_frame_ready` / `warm_up`, `ensure_extent_engine`, 상주 `auto_fit_all`, `pick_point` / `pick_data`가 실행되는 동안 연결 세대와 커널을 확인하는 `busy` 토큰을 유지한다. 그동안 `frame()`·포인터·크기 변경·대리 호출은 생략하거나 거부한다. 상주 `auto_fit_all`은 완료 시 설정을 직접 바꾸므로 실행 중 `frame()`을 호출할 수 없다.
- 웹 래퍼의 스트리밍 `auto_fit_all()`은 위 저수준 비동기 메서드를 호출하지 않는다. 렌더러에 범위 계산을 요청하고 스트리밍 작업의 `done`을 기다린다. 청크 공급과 커널 호출을 계속해야 하므로 일반 `busy` 토큰을 잡지 않으며, 작업 중 `busy`가 `false`일 수 있다. 스트리밍 실행기가 변경 요청을 최신 상태에 맞춰 반영한다.
- 웹 래퍼는 작업 중 마지막 크기 변경과 포인터 해제만 보관한다. 작업이 끝나면 이를 WASM에 적용한 뒤 토큰을 해제한다. 연결을 끊어도 사용 중인 커널은 작업 종료 후 해제한다. 이전 연결 세대의 작업이 끝났다고 새 세대의 토큰을 해제해서는 안 된다.

웹 래퍼 사용 예제는 `crates/web/index.html`에 있다. 저수준 커널은 위 규칙을 직접 구현해야 하는 경우에 사용한다.

## 4. 제약과 주의사항

- **메인 스레드 사용**: 축·장식의 CPU 렌더링은 메인 스레드에서 실행한다. 기존 측정에서는 글리프 캐시 적용 후 600×460 패널의 release 빌드에서 프레임당 약 0.4ms였으며, 실제 비용은 환경과 차트에 따라 달라진다. 더 큰 작업은 `OffscreenCanvas`와 Web Worker로 옮길 수 있다. SharedArrayBuffer를 쓰는 WASM 스레드를 사용하려면 서버의 COOP/COEP 헤더로 교차 출처 격리를 설정해야 한다.
- **WebGPU 지원**: HTTPS나 루프백 주소에서 `navigator.gpu` 존재 여부와 어댑터·장치 요청 성공을 확인한다. 지원 여부는 브라우저 버전뿐 아니라 OS·GPU·드라이버·브라우저 정책에 따라 다르다. 이 렌더러는 컴퓨트·스토리지 버퍼와 WebGPU 초기화를 사용하므로 wgpu의 `webgl` 기능만 켜서 WebGL2로 대체할 수 없다. 일반 환경에서는 브라우저의 기본 GPU 선택을 따른다. 실제 GPU가 없는 Linux 검증 환경에서만 별도 실행 설정으로 SwiftShader Vulkan을 사용할 수 있다.
- **폰트**: 내장 Liberation Sans에는 한중일 문자가 없다. 한글을 표시하려면 `register_font(Uint8Array)`로 TTF/OTF 파일을 등록하고 반환된 글꼴 이름을 `font`에 지정한다. fontdb는 WOFF2를 읽지 못한다. 스케치 모드는 내장 Comic Neue(OFL)를 사용한다. 이 폰트에 없는 한중일·그리스 문자 등은 글자별로 등록 폰트와 Liberation Sans에서 찾아 그린다. 따라서 등록한 한글 폰트는 스케치 모드에서도 사용할 수 있다.
- **동기 대기 금지**: WASM에서 `pollster::block_on`을 사용하면 교착 상태가 발생할 수 있다. 구현은 비동기로 작성하고 동기 래퍼는 `#[cfg(not(target_arch = "wasm32"))]`로 네이티브에서만 빌드한다.

## 5. 빌드 산출물 — `crates/web` → `pkg/`

브라우저 패키지의 소스는 `crates/web`에 있으며 패키지명은 `figgy`다. 다음 명령으로 release 빌드를 만든다.

```bash
npx wasm-pack@0.15.0 build crates/web --release --target web --locked
```

생성 파일은 `crates/web/pkg/`에 저장하며 프론트엔드에 함께 배포한다.

| 파일 | 내용 |
|---|---|
| `figgy_bg.wasm` | WASM 본체. wgpu, Liberation Sans 4종, Comic Neue 2종을 포함한다. 크기는 툴체인·기능·최적화에 따라 달라진다. |
| `figgy.js` | WASM과 JavaScript를 연결하는 ES 모듈 — `import init, { FiggyChart, … }` |
| `figgy.d.ts` | 자동 생성한 TypeScript 타입 정의. 저수준 WASM API 형식을 확인할 수 있다. |
| `package.json` | npm용 패키지 메타데이터 |

일반 웹 API는 `crates/web/figgy-chart.js`가 제공한다. `pkg/`에서 생성되는 파일이 아니므로 함께 배포해야 한다. 이 파일은 내부 캔버스, rAF, DPR·포인터 좌표 변환, 선택·드래그·크기 조절, ResizeObserver, 준비 이벤트와 비동기 호출 직렬화를 담당한다. 사용 예제는 `crates/web/index.html`, 전체 JSON 설정은 [SCHEMA.md](../web/SCHEMA.md)를 참고한다.

로컬에서는 HTTP 서버로 열어 확인한다.

```bash
cd crates/web && python -m http.server 8137   # wasm은 file:// 불가
```

`<figgy-chart>`의 공개 API는 다음과 같다.

| 분류 | 메서드와 동작 |
|---|---|
| 객체 수명 | `ready` Promise, `figgy-ready` / `figgy-init-progress` / `figgy-error` / `figgy-select` / `figgy-drag` / `figgy-release` / `figgy-resize` 이벤트, `free()`. 첫 프레임과 준비 완료 알림 후 백그라운드 피킹 준비를 시작한다. 실패하면 `operation: "prewarm_gpu_picking"`, `recoverable: true`인 `figgy-error`를 보내며 준비 완료 상태는 유지한다. |
| 폰트 | `register_font(Uint8Array)`는 TTF/OTF/TTC를 등록하고 글꼴 이름 배열을 반환한다. 반환된 이름을 `font`에 사용할 수 있다. 등록 폰트가 시스템 폰트보다 우선한다. 내용이 같은 파일은 중복 저장하거나 폰트 세대를 늘리지 않으며 글꼴 ID별 데이터를 재사용한다. 찾을 수 없는 글꼴은 내장 Liberation Sans로 대체한다. 한글은 별도 폰트 등록이 필요하다. |
| 스타일 옵션 | 독립 함수 `draw_style_modes()`는 모드 이름의 JSON 배열을, `draw_style_param_specs(mode)`는 `{key, min, max, default, integer}`의 JSON 배열을 반환한다. UI 슬라이더를 만들 때 이 값을 사용한다. 최소·최대는 권장 범위이며 설정은 바깥 값도 허용하고 렌더러가 안전상 필요한 제한만 적용한다. 기본값은 모델의 `Default`와 테스트로 일치시킨다. |
| 컬럼 등록·교체·제거 | `register_column_f32/f64(id, TypedArray)`는 새 ID를 등록한다. `register_columns_f32/f64(ids, TypedArray, valuesPerColumn)`은 연속된 배열로 새 컬럼을 한 번에 등록하며 전체 성공 또는 전체 실패한다. `update_register_column_f32/f64(id, TypedArray)`는 기존 ID를 매번 업로드해 교체한다. `remove_column(id)`로 제거한다. |
| 시리즈 | `add_line_series(id, x, y, width, label)`은 추가 또는 교체, `remove_series(id)`는 제거다. |
| 범례 | `set_series_label(id, label)`로 텍스트를 바꾼다. 줄바꿈과 유니코드 첨자를 지원하며 빈 문자열이면 해당 행을 지운다. `set_series` / `apply_color_cycle`은 인식 가능한 자동 기호만 갱신하고 사용자 텍스트는 보존한다. 전체 재생성은 `reset_legend_from_series_labels()`를 호출한다. 직접 편집할 때는 `legend.content`의 줄바꿈·탭·고정 너비 기호·구간별 색을 사용한다. 기호의 `field_em`은 2.0em이며 선은 `rule:true`, 점선 패턴은 `rule_dash`로 지정한다. `content.font` / `font_size`와 구간별 설정은 그리기 시 적용한다. |
| 위치 판정 | `hit_test(x, y)`는 요소 ID 또는 `null`을 반환한다. 데이터 영역·축·눈금 라벨·제목·색상 막대와 그 세부 요소를 구분한다. 선택 상태 자체는 바꾸지 않는다. 위치는 렌더러의 레이아웃에서 계산하므로 호스트가 별도로 상자를 관리할 필요가 없다. |
| 데이터 피킹 | `pick_point(x, y, max_distance_px)`는 `Promise<{ source_id: string \| null, series_id, point_index, distance_px } \| null>`을 반환한다. 점은 스타일을 반영한 기호 크기, 선은 선분과의 거리로 판정하고 가까운 끝점을 고른다. 오차 막대 몸통·끝선은 대상이 아니다. 좌표는 반환된 인덱스로 원본 컬럼에서 조회한다. |
| 전체 사전 준비 | `prewarm_all_with_progress(callback)`은 상주 렌더링·스타일·경로 길이·범위·피킹·등고선용 캐시를 생성하고 `{ scope, stage, phase }`를 알린다. `prewarm_all()`은 콜백 없이 같은 작업을 한다. 둘 다 연결 세대를 확인하는 작업 직렬화 장치를 사용한다. `warm_up()`은 첫 프레임 대기용 호환 메서드다. |
| 피킹 준비 | 준비 완료 후 백그라운드에서 실행한다. `prewarm_gpu_picking()`으로 미리 준비하거나 재시도할 수 있다. 저수준 API와 래퍼 모두 렌더러의 같은 파이프라인과 차트 캐시를 사용한다. |
| 범위 맞춤 | `auto_fit_all(pad)`는 모든 시리즈의 원본 도형을 포함하는 X·Y 범위를 구하고 비율 여백을 더한다(0은 여백 없음, 0.05는 5%). 상주 선·점·오차 막대·행렬은 GPU로 범위를 계산하며 히스토그램은 경계·빈도 메타데이터와 기준선을 사용한다. 상주 경로는 GPU 결과를 읽어 설정에 반영할 때까지 다른 커널 호출을 막는다. 래퍼의 스트리밍 경로는 일반 busy 토큰 없이 축 반영과 렌더링 완료를 기다린다. 저수준 스트리밍 호출은 요청만 등록하므로 호스트가 작업을 계속 실행해야 한다. 범위 끝은 반올림하지 않는다. 눈금은 범위 안에서 읽기 좋은 값으로 배치한다. `auto_fit_colorbar(pad)`는 상주 행렬의 Z값 통계로 색상 막대를 맞춘다. `auto_fit_x/y(col, pad)`는 단일 컬럼 통계를 사용하며 오차 막대를 반영하지 않는다. `load_demo()`는 데모를 불러온다. |
| 행렬·막대 정보 | `set_contour_nice_levels(series_id, target_count, use_colormap_colors)`는 색상 막대 눈금 규칙으로 등고선 값을 만들고 시리즈 설정에 기록한 뒤 레벨 수를 반환한다. `series_draw_info(series_id)`는 `{ drawn_count, cols, rows, truncated }`를 반환한다. 저수준 WASM은 JSON 문자열, 래퍼는 객체를 반환한다. |
| 피킹 기준 | 최종 장식 픽셀 대신 원본 점·선분을 기준으로 한다. 산점도는 지정한 기호 반지름을, 선은 인접 원본 점 사이의 직선을 사용한다. 점선의 공백, 사각 끝 모양, 스케치의 흔들림은 판정 경로에 영향을 주지 않는다. |
| 설정 읽기·쓰기 | `get_config()` / `set_config(json)`, `get_series()` / `set_series(json)`. `set_colorbar_axis(json)`은 색상 막대의 전체 `AxisOptions`를 교체한다. |
| 프리셋 | `apply_axis_preset(AxisPreset)`, `apply_color_cycle(ColorCycle)`, `color_cycle_css(cycle)`. |
| 포인터 입력 | 웹 래퍼가 포인터 이벤트를 처리한다. 직접 전달할 때는 `on_press(x, y)`, `on_move(dx, dy)`, `on_release()`, `has_selection()`을 사용한다. |
| 선택 표시 | `set_picked_points(json)`은 `PickedPointsConfig` 또는 `null` JSON 문자열로 `Config.picked_points`만 교체한다. `set_picked_data(json)`는 점·막대·셀·등고선 선택을 같은 방식으로 바꾼다. `null`은 표시를 지운다. 출처·인덱스만 보관하며 원본 좌표의 CPU 복사본은 만들지 않는다. |
| 배경 | `set_clear_color(r, g, b, a)`는 선형 RGBA를 성분별 0~1로 제한하고 다시 그리기를 요청한다. 화면 상태만 바꾸므로 Config JSON이나 축 이미지는 갱신하지 않는다. |
| 출력 | `export_png(scale)`은 PNG의 `Uint8Array`를 비동기로 반환한다. |
| 제목 | `set_title`, `set_x_title`, `set_y_title`, `set_colorbar_title`. 색상 막대 제목에 빈 문자열을 주면 숨긴다. |

점별 스타일은 `set_series(json)` / `get_series()`로 설정한다. 정밀 모드의 산점도는 `point_style_table` / `point_style_index_column` / `point_style_overrides`를, 오차 막대는 `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`를 사용한다. 별도 WASM 메서드를 추가하지 않으며 다른 렌더링 스타일에서는 이 매핑을 무시한다.

`element.kernel`로 저수준 WASM `FiggyChart`에 직접 접근할 수 있다. 이 경로는 웹 래퍼의 작업 중 접근 제한과 수명 관리를 거치지 않으므로 디버깅이나 별도 통합을 직접 구현할 때 사용한다.
<a id="exact-streaming"></a>

<a id="공개-후보-자동-실행과-원본-구간-공급"></a>

### 자동 실행과 원본 구간 공급

아래 API는 figgy 0.10.0부터 제공한다. `render_chart()`가 작업 실행을 관리하므로 앱에서 `auto_stream_chart_step()`이나 `frame()`을 반복 호출할 필요가 없다. 네이티브 Rust에서 빌린 `ColumnSource`를 사용하는 방식과 청크 실행 API는 그대로 유지한다.

![figgy의 상주 및 비상주 화면 렌더링 구조](assets/streaming-architecture-en.png)

그림은 공통 렌더러의 상주 경로, 스트리밍 누적 이미지, 선택적인 GPU 패킹 캐시, 원본을 다시 읽는 출력을 보여 준다. 브라우저에서는 다음과 같이 역할을 나눈다.

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
chart.configure_streaming(1, 2, 8, 2 * 1024 ** 2, 16 * 1024 ** 2);
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
if (result.status !== "complete" && result.status !== "resident") {
  throw new Error(`stream ${result.status}`);
}
const png = await chart.export_png(2);
// 화면을 버릴 때: await job.cancel();
```

#### 원본 공급과 작업 실행

- `columns`에는 필요한 수만큼 컬럼을 지정할 수 있다. 전체 배열을 갖고 있다면 `{ id, revision, values: typedArray }`를 전달하고 `readRange`를 생략한다. 웹 래퍼는 배열 참조만 보관하고 필요한 구간만 WASM의 쓰기 전용 스테이징에 기록한다. Worker에서 만든 구간은 전송 가능한 버퍼로 넘길 수 있다. 파일 파싱·원본 저장·Worker 입출력은 호스트가 담당한다.
- 작업이 끝나도 원본 참조나 `readRange`를 유지한다. 크기·DPR 변경과 이미지 출력에 같은 리비전의 원본이 필요하기 때문이다. 같은 리비전의 값을 수정하거나 버퍼를 분리(detach)하지 않는다. 수정하면 새 리비전으로 등록한다. `job.cancel()`이나 컴포넌트 해제 시 해당 작업이 보관하던 참조를 놓는다.
- 설정·시리즈·소스 리비전·처리 위치는 렌더러가 관리한다. 웹 래퍼는 구간 요청과 완료 알림을 연결하며 GPU 계산을 중복 구현하지 않는다. 데이터 축소나 LOD도 적용하지 않는다.
- `maxFrameTimeMs`는 실제 동기 제출 시간에 맞춰 청크 크기를 조절하기 위한 목표값이다. 브라우저 스케줄링·데이터 공급자·GPU 명령의 실행 시간을 강제로 제한하지는 않는다. 작업은 MessageChannel과 GPU 완료 알림으로 진행하고 화면 표시는 rAF에 맞춘다.
- `job.done`은 완료·상주·취소·대체 상태를 반환하며 실패하면 Promise가 거부된다. `job.cancel()`은 제출한 GPU 작업의 자원 회수까지 기다린다. 실행 중인 GPU 명령을 강제로 중단하지는 않는다. 이전 작업의 데이터 응답이 늦게 와도 새 작업에 반영하지 않는다.
- 일반 점·선의 원본 구간 업로드는 GPU 작업 버퍼와 제한된 수의 staging 슬롯을 재사용한다. 슬롯 수는 `max_slots`를 넘지 않으며, GPU 복사와 다시 매핑하는 작업이 끝나지 않았으면 `backpressure`로 기다린다. 호스트는 GPU 완료를 기다리고 브라우저 이벤트 루프에 실행 기회를 줘야 한다. 초기 fit으로 다시 그릴 때도 슬롯을 유지한다. 선택 표시·격자·경로 길이 계산·export의 별도 업로드 자원에는 같은 재사용 방식을 일괄 적용하지 않는다.
- 취소·교체 등으로 마지막 공유 소유자가 자원을 놓으면 회수 대기 상태로 옮긴다. 기존 명령의 제출 경계를 지난 뒤 GPU 완료를 확인하고 추적 중인 버퍼·텍스처에 `destroy()`를 호출한다. 사용 중인 버퍼를 청크마다 파괴하는 동작이 아니며, JavaScript GC가 실행될 때까지 반환을 미루지 않기 위한 처리다.
- `stream_status()`는 상태만 조회한다. 완료 후에도 누적 도형 수·리비전·작업 ID를 유지한다. 같은 입력을 다시 요청하면 원본을 다시 읽거나 그리지 않는다.

<a id="streaming-fit"></a>

#### 첫 스트림의 범위 맞춤과 이후 조작

**스트리밍 자체는 자동 맞춤을 켜지 않는다.** 새 차트의 표시 범위를 모르면 호스트가 첫 청크 전에 한 번 요청한다. 렌더러는 원본 구간을 앞으로 읽으면서 누적 범위를 계산한다. 새 청크로 범위가 넓어지면 앞서 그린 데이터 이미지를 GPU에서 리스케일하고 새 청크를 이어 그린다. 축과 눈금은 새 범위로 따로 그리므로 이전 눈금이 이미지와 함께 늘어나거나 잔상으로 남지 않는다.

숫자 눈금 라벨은 현재 축 범위에서 실제 글자 크기와 화면 길이를 재어, 서로 겹치지 않는 일정한 간격으로 표시한다. 화면을 넓히거나 확대하면 생략했던 숫자가 다시 나타난다. 글꼴 크기와 설정의 주 눈금·격자 간격은 유지하며, 지수 표기·반전 축·색상 막대에도 같은 규칙을 적용한다. PNG 배율에 맞춰 글자 사이 여백도 함께 커진다.

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

진행 중인 자동 맞춤 요청은 요청 당시의 설정과 시리즈에 속한다. `set_config()`·`set_series()`로 설정을 복원하거나 작업을 취소하면 이전 요청을 해제한다. 같은 WASM 객체로 다른 차트를 표시해도 이전 차트의 맞춤 요청을 이어받지 않는다. 다시 맞춰야 한다면 설정과 시리즈를 모두 적용한 뒤 명시적으로 요청한다.

창 크기나 DPR이 바뀌면 실제 캔버스 픽셀 크기와 `resize(width, height)`를 함께 갱신한다. 저수준 스트리밍 호스트는 `request_auto_streaming_chart()`를 다시 호출해 새 크기의 표시 타깃을 요청하고 완료까지 진행한다. 이때 `auto_fit_all()`은 호출하지 않는다. 저장된 축 범위와 문서 크기는 유지하고, 글자·테두리·최종 데이터를 새 해상도로 그린다. 작은 캔버스를 CSS로 늘린 상태를 최종 화면으로 사용하지 않는다.

자동 맞춤은 현재 데이터로 범위를 계산하는 동작이다. 데이터와 여백이 같으면 처음 맞춘 범위로 돌아가지만, 데이터가 바뀌었다면 새 데이터에 맞춘다. `padding`은 유한한 0 이상의 수이며, `0`은 여백 없음, `0.05`는 5% 여백이다. 범위 계산과 통계 재사용은 렌더러가 담당한다.

현재 `Config`의 축 `min`·`max`는 숫자다. 기본값만 보고 새 차트인지, 사용자가 그 범위를 선택했는지 구분할 수 없으므로 생성·불러오기 흐름에서 자동 맞춤 여부를 정한다. 이 구분을 위해 `null`이나 생략한 범위를 전달하지 않는다. `Option<AxisRange>`나 `RangePolicy`는 현재 API에 없다.

**웹 래퍼(`<figgy-chart>`)**에서는 위 예제처럼 `render_chart()`로 원본 공급자를 연결한 직후, 같은 동기 호출 구간에서 `auto_fit_all()`을 호출한다. `await job.done` 뒤에 요청하면 첫 렌더링이 기존 범위로 진행된다. 반대로 `readRange` 공급자를 연결하기 전에 맞춤을 요청하면 재공급할 원본을 찾지 못할 수 있다. 두 Promise를 함께 기다리고, 취소·대체된 작업의 결과를 현재 차트에 반영하지 않는다.

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

이 절의 리스케일 미리보기·최종 화면 교체·버퍼 재사용 수정은 `renderer 0.12.2` / `figgy 0.10.2`에 포함된다. 이전 버전에서 같은 API를 사용했더라도 소스 리비전을 갱신하고 WASM을 다시 빌드해야 한다. API 형식과 Config JSON 스키마는 바뀌지 않았다.

#### 화면 변경, 선택과 출력

- 데이터나 표시 범위가 바뀌면 다음 작업 단계에서 최신 상태로 교체한다. 제목 같은 장식만 바뀌면 데이터 처리 위치와 누적 이미지를 유지한다. 크기·DPR 변경 시에는 새 픽셀 해상도로 원본을 다시 그린다.
- `await auto_fit_all()`은 통계로 계산한 축 범위의 반영과 렌더링 완료를 기다린다. 일반 비동기 호출의 `busy` 토큰을 잡지 않으므로 스트리밍 실행은 계속 진행된다. 통계는 최초 구간 업로드 때 수집하고 렌더러가 리비전별로 재사용한다.
- 첫 스트림과 자동 맞춤 버튼의 호출 순서는 [범위 맞춤 사용법](#streaming-fit)을 따른다. 일반 재그리기는 자동 맞춤을 켜지 않는다.
- 선택만 바뀌면 데이터 스트림을 다시 시작하지 않는다. 선택한 원본 구간만 요청하고 새 GPU 자원이 준비될 때까지 이전 표시를 유지한 뒤 한 번에 교체한다. 공급이 실패해도 이전 표시를 유지하고 `figgy-error`로 알린다. 취소되거나 다른 선택으로 대체된 요청의 지연 응답은 무시한다. 출력은 시작 당시의 선택을 사용하므로 이후 화면 선택과 섞이지 않는다.
- 완료된 패킹 캐시가 있으면 `pick_point` / `pick_data`가 GPU의 점·선에서 원본 행 인덱스를 반환한다. 원본을 다시 읽지 않는다. `next_view_point_index(sourceId, seriesId, current, forward)`는 캐시에 남아 있는 행 중 다음·이전 원본 인덱스를 동기적으로 반환하며, 대상이 없으면 `null`이다. 이 조회로 GPU 피킹이나 공급자 요청을 다시 실행하지 않는다.
- 패킹 캐시가 없거나 아직 준비 중인 비상주 스트림의 피킹은 즉시 `null`을 반환한다. 저수준 WASM의 `begin_stream_pick_*` / `finish_stream_pick_*`는 호환성을 위해 함수 형식만 남겨 두며 즉시 오류를 반환한다. 상주 차트의 피킹은 계속 지원한다. 이미 아는 인덱스의 선택 표시는 필요한 원본 구간만 읽어 처리한다.
- `export_png`는 문서 크기와 출력 배율에 맞춰 원본을 다시 그린다. 화면 텍스처를 늘리지 않으며 화면 작업의 처리 위치나 누적 이미지는 바꾸지 않는다.

#### GPU 캐시와 메모리 한도

- `inspect_column_admission(metadata)`는 전체 컬럼을 명시적으로 등록할 때 필요한 인코딩 크기·한도·거부 이유를 조회한다. 공간을 예약하거나 모든 파생 자원의 할당을 보장하는 호출은 아니며 자동 스트리밍의 실행 기준도 아니다.
- `configure_auto_residency()`는 전체 GPU 예산과 차트별 GPU 캐시 한도를 정한다. 지원되는 정밀 모드의 점·실선·오차 막대 차트는 스트리밍 중 현재 화면에 필요한 원본 행만 GPU에 남긴다. 완성된 패킹 페이지는 GPU로 옮기고 CPU에는 현재 청크와 작성 중인 페이지만 둔다. 화면 밖 점을 잇지만 화면을 통과하는 선과 화면에 닿는 오차 막대의 원본 행도 포함한다. 상주 한도 0은 패킹 캐시를 끈다. 연결된 컬럼 전체를 자동으로 `ColumnPool`에 넣지는 않는다.
- 표시 범위를 좁히거나 창 크기·비율·DPR을 바꿔도, 새 화면의 필요 영역이 캐시에 있으면 패킹된 원본 행만으로 다시 그린다. 캐시에는 선 두께·점 크기·오차 막대 끝과 AA 영역에 더해 가장자리 8픽셀의 여유를 포함한다. 이 추가 데이터도 기존 패킹 한도에 포함된다. 화면 크기의 일치 여부 대신 데이터 좌표에서 실제 포함 관계를 검사하며, 새로 필요한 가장자리까지 캐시가 덮지 못하면 원본 구간을 다시 요청한다. 지원하지 않는 도형, 패킹 한도 초과, GPU 예산 부족, 할당 실패 시에는 원본을 빠짐없이 그리는 스트리밍 경로를 유지한다. `stream_status().view_residency`의 `state`, `needed_bytes`, `refusal_reason`, `picking_available`로 상태를 확인한다. `needed_bytes`는 준비 중에는 지금까지 필요한 최소 바이트 수, 완료 후에는 실제 패킹 크기다. `job.cancel()`은 해당 작업과 표시 자원을 정리한다.
- 점선·히스토그램·히트맵·스케치·별자리와 지원되는 은하수 스트림에는 원본 순서와 인접 관계를 보존하는 별도 GPU 캐시를 사용한다. 참조 컬럼 전체의 hi/lo 인코딩 크기 합계가 차트별 상주 한도와 장치의 단일 버퍼 한도에 들어오고 전체 GPU 예산에도 여유가 있을 때만 만든다. 청크를 올릴 때 GPU 안에서 복사하며 CPU 원본 복사본은 남기지 않는다. 크기·DPR·축 범위·그리드·히트맵 색상 매핑 변경은 캐시된 원본으로 다시 계산하므로 이미지 확대에 따른 흐림이나 이전 설정의 잔상이 생기지 않는다. 캐시에 없는 구간, 새 데이터 리비전, 예산 초과는 일반 스트리밍으로 처리한다. 큰 컬럼의 일부만 임의로 잘라 이 캐시에 넣는 최적화는 아직 하지 않는다. 원본 공급자는 출력과 캐시 미적중에 대비해 계속 보관해야 한다.
- 원본 구간 캐시는 데이터 재공급 비용을 줄이는 기능이다. `view_residency`는 기존의 패킹된 뷰와 즉시 피킹 가능 여부를 나타내므로 이 캐시만 있는 차트는 여전히 `streamed`로 표시한다. 상주 한도를 0으로 내리면 원본 구간 캐시도 해제한다. 네이티브 `auto_stream_chart_request_ranges()`는 캐시 적중 시 한 청크의 GPU 작업을 진행하고 `Backpressure`를 반환할 수 있다. 기존처럼 완료까지 계속 호출하고 `Ready`일 때만 원본을 공급한다. 조회만 필요하면 작업을 진행하지 않는 `stream_status()`를 사용한다.
- 웹 컬럼 풀의 기본 크기는 16MiB다. `set_pool_auto_growth(true)`로 자동 확장을 켜면 업로드 공간이 부족할 때 늘린다. 기본값은 `false`이며 장치의 스토리지 바인딩 한도와 전체 GPU 예산을 넘지 않는다. 할당 오류는 JS `Error.code`의 `pool_space`, `budget_exceeded`, `device_limit`, `allocation_failed`로 구분한다. 가능한 경우 `requestedBytes`, `limitBytes`, `largestFreeBytes`, `totalFreeBytes`도 제공한다. JavaScript가 정수로 정확히 표현할 수 있는 범위 안에서는 숫자, 그 밖에서는 10진 문자열로 반환한다.
- `gpu_memory_status()`는 요청 할당량의 사용 중·회수 대기·합계(`live/retired/total`), 풀 용량·점유·최대 빈 공간·백업·반환 대기량, 자원별 사용량, 컬럼별 `resident` / `streamed` 상태를 반환한다. 실제 VRAM 잔량을 측정하지는 않는다. 은하수·별자리 스타일의 미리 생성한 텍스처는 일반 집계에서 제외되며 스트리밍 출력의 실행 가능 여부를 검사할 때만 크기를 따로 고려한다.
- `await release_unused_gpu_memory()`는 활성 스트림이 없을 때 남은 컬럼을 GPU 안에서 작은 풀로 옮기고 이전 버퍼의 작업 완료를 기다린다. 최소 16MiB는 유지한다. 축소용 버퍼도 잠시 필요하므로 예산·장치 한도·할당 오류가 발생할 수 있다. 실패하면 기존 컬럼과 바인딩을 유지한다.
- 호스트 정책의 예로 전체 예산 `4_000_000_000`바이트, 뷰별 패킹 한도 `500_000_000`바이트를 둘 수 있다. 현재 화면의 캐시가 한도를 넘으면 스트리밍을 유지한다. 컬럼 등록 시 공간·예산이 부족하면 불필요한 컬럼 제거, 정리 완료 대기, 한 번 재시도 순서로 처리한다. 계속 실패하거나 장치 한도·실제 할당 오류가 나면 스트리밍으로 전환한다. 이 수치는 예시이며 렌더러의 기본값이 아니다.

정밀 스타일의 스트림 누적 화면에는 데이터만 저장한다. 그리드는 표시할 때 아래에 합성하므로 표시 여부·색·두께·선 모양을 바꿔도 작업 ID와 데이터 처리 위치를 유지한다. 상주 캐시가 없는 스트림도 원본 재생 없이 그리드를 갱신한다. 배경에 의존하는 다른 스타일은 그리드를 바꾸면 데이터를 다시 그린다. 원본 구간 GPU 캐시가 있으면 호스트에 같은 청크를 다시 요청하지 않는다. 정밀 스타일의 분리 합성은 텍스처를 리샘플링하거나 AA를 다시 적용하는 과정이 아니다. 다만 UNORM 색상 합성과 MSAA 반올림 때문에 배경 위에 직접 그리는 상주 경로와 색상 채널이 비트 단위로 같다는 보장은 없다. 회귀 장면에서는 채널 차이를 8비트 값 기준 최대 2로 제한하며, 반투명 점·막대 채움·테두리가 겹치는 장면은 최대 3으로 검사한다.

`crates/web/tests/view-cache-resize-grid-probe.html`은 창 크기·비율 변경 시 원본 요청이 없는지, 새 크기에서 처음부터 스트리밍한 화면과 픽셀이 일치하는지, 그리드 변경이 작업 재시작 없이 반영되는지 검사한다.

#### 지원 범위

- 비상주 히트맵은 그리기·선택 표시·출력을 지원하지만 즉시 피킹은 지원하지 않는다. 이미 아는 셀을 표시할 때는 경계 계산에 필요한 인접 축 값만 읽는다. 자동 범위 맞춤은 기존 GPU 격자 계산을 사용하고 결과를 리비전별로 저장한다.
- `streaming_capabilities()`는 현재 설정과 시리즈의 지원 여부·이유를 반환한다. `operations_require_completed_revision`은 조회·출력 전에 작업 완료가 필요한지를 나타낸다. 지원하지 않는 조합은 무시하거나 다른 스타일로 바꾸지 않는다. 등고선과 은하수의 스트리밍 선·별 연결은 지원하지 않는다. 정밀·스케치·별자리의 점선은 기존 경로 길이 스캔의 연산 순서를 유지하므로 청크가 바뀌어도 점선 간격이 처음부터 다시 시작되지 않는다.

브라우저 회귀 검증 페이지는 `crates/web/tests/streaming-contract-probe.html`이다. 430만 점, Worker 구간 공급, 상주 차트 동시 표시, 크기 변경·취소·범위 맞춤, 패킹 캐시 없는 스트림 피킹의 즉시 종료와 원본 무조회, 1배·2배 출력을 검사한다.

`pick_point`의 JSON·객체·`null` 반환 형식과 Promise 오류 전달 방식은 0.8에서도 같다. 제출된 요청은 결과 읽기 자원과 당시의 `Arc` 기반 `source_id` / `series_id` 매핑을 보관한다. 대기 중 차트·풀이 바뀌거나 렌더러가 해제돼도 해당 요청의 대상은 바뀌지 않는다. 점 좌표의 CPU 복사본도 만들지 않는다.

<a id="등록해제-모델--메모리는-내부-자동-관리"></a>

### 데이터 등록·제거와 메모리 관리

차트는 캔버스마다 인스턴스 하나를 두고 ID로 데이터를 등록·교체·제거한다. 풀의 할당, 재배치와 자원 해제는 렌더러가 처리한다.

- `register_column_f32/f64(id, data)`는 새 ID만 받는다. 이미 있으면 오류다.
- `register_columns_f32/f64(ids, data, valuesPerColumn)`도 새 ID만 받는다. 행렬용 일괄 등록이며 배치 전체를 한 번에 업로드한다. 실패하면 어떤 컬럼도 등록하지 않는다.
- `update_register_column_f32/f64(id, data)`는 기존 ID만 받는다. 유효한 호출은 같은 값이라도 매번 업로드하며 실패하면 기존 상태를 유지한다. 해시만 비교해 업로드를 생략하지 않는다.
- `set_series(json)`는 등록된 컬럼 중 그릴 대상을 지정하며 데이터를 업로드하지 않는다.
- 업로드할 때 자동 범위 맞춤용 최소·최대·최소 양수 값을 저장한다. 점선 간격에 필요한 누적 경로 길이 같은 점별 정보는 GPU 스캔(`line_arc.wgsl`)으로 계산한다.
- 오차 막대 방향은 `PrimitiveStyle::primitive_flags`의 Y=bit 0, X=bit 1로 지정한다. 사용하지 않는 정점 슬롯에는 기준점 컬럼을 다시 바인딩하고 셰이더가 해당 속성을 읽기 전에 처리를 생략한다. 빈 값을 채운 별도 컬럼이나 예약 ID는 만들지 않는다. 웹 래퍼도 이를 위한 길이 계산·메타데이터 복제·추가 업로드를 하지 않는다.
- `remove_column(id)`는 해당 컬럼을 참조하는 시리즈도 제거한다. 자동 범례에서는 대응하는 행을 지운다. `set_config`로 직접 편집한 범례는 사용자 텍스트를 보존하고 남은 시리즈의 인식 가능한 기호만 갱신한다.
- 제거·교체로 생긴 빈 공간은 렌더러가 정리 작업으로 등록하고 다음 `frame()` 시작에 GPU 내부 복사로 모아 정리한다. 연속 교체 중 할당 공간이 부족하면 `OnAllocFailure` 정책으로 재배치를 시도한다. 웹은 별도의 재배치 플래그나 피킹 재바인딩 상태를 관리하지 않는다.
- `add_line_series`는 기존 ID이면 색을 유지하며 교체하고, 새 ID이면 색상 순서의 다음 색을 사용한다. 기존 ID에 빈 라벨을 전달해도 기존 범례 텍스트는 지우지 않는다. 비어 있지 않으면 해당 행의 텍스트만 바꾼다.
- `free()`로 인스턴스를 해제하면 풀 버퍼·파이프라인·텍스처·surface의 참조를 정리한다. GC의 FinalizationRegistry도 정리를 지원하지만 실행 시점이 정해져 있지 않으므로 SPA에서 컴포넌트를 제거할 때는 `free()`를 직접 호출한다.

현재 `wasm-opt`는 꺼져 있다. wasm-pack에 포함된 Binaryen이 최신 rustc 출력의 일부 기능에서 비정상 종료되기 때문이다(`crates/web/Cargo.toml` 참고). Rust의 release 최적화는 적용한다. 추가 용량 절감이 필요하면 호환되는 최신 Binaryen으로 검증한 뒤 다시 활성화할 수 있다.
