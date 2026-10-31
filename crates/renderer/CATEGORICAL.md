# 범주형 막대 차트

`CategoricalChart`는 범주·시리즈·값을 직접 보관하는 소규모 모델이며, 화면·선택·내보내기가
모두 이 모델을 단일 원본(SSOT)으로 삼는다. 숫자 컬럼용 `ColumnSource`와 별도 계약이며 스트리밍을
사용하지 않는다. 범주형 막대 API는 crates.io의 `figgy-renderer` 0.16.1부터 들어 있다.
`CategoricalRenderer::new_async`는 0.17.0부터, [주석 층](#주석-층)과 차트의
`chart_title`·`active_layer` 필드는 0.18.0부터 쓸 수 있다.

```rust,no_run
use renderer::{CategoricalChart, Category, BarSeries, Color, CategoryBarMode};
let chart = CategoricalChart {
    categories: vec![Category::new("a", "A"), Category::new("b", "B")],
    series: vec![
        BarSeries::new("before", "Before", vec![Some(12.0), Some(8.0)], Color::from_rgb8(70,130,205)),
        BarSeries::new("after", "After", vec![Some(18.0), Some(15.0)], Color::from_rgb8(255,166,105)),
    ],
    mode: CategoryBarMode::Grouped,
    ..Default::default()
};
chart.validate()?;
# Ok::<(), &'static str>(())
```

## 입력과 누적 규칙

- 범주 1~64개, 시리즈 1~16개, `범주 수 × 시리즈 수`는 최대 512다.
- 범주·시리즈 ID는 각 목록에서 고유해야 한다. 이름은 중복될 수 있고, 이름을 바꿔도 ID는 바뀌지 않는다.
- 각 시리즈의 `values` 길이는 범주 수와 같아야 한다. `None`은 누락, `Some(0.0)`은 실제 0이다.
  둘 다 막대 면적은 없지만 0은 값·표기·선택 상태에 남는다. 포인터로는 면적이 있는 막대만 선택한다.
- `Grouped`는 나란히 그린다. 시리즈가 하나면 단일 막대다. 누락되어도 다른 막대의 자리는 이동하지 않는다.
- `Stacked`는 양수를 위쪽/오른쪽으로, 음수를 아래쪽/왼쪽으로 각각 0부터 쌓는다.
- `PercentStacked`는 범주별 양수 합계를 100%로 환산한다. 음수는 거절한다. 합계 0은 빈 막대이며
  퍼센트 표기는 `—`다. 원본 값은 바꾸지 않는다. 큰 값도 정규화한 뒤 합산해 합계 오버플로를 피한다.
- 일반 누적에서 합계가 유한한 숫자 범위를 벗어나면 거절한다. NaN·무한대는 모든 모드에서 거절한다.
- `Percent` 표기는 같은 범주의 시리즈 합계가 기준이다. 음수가 섞인 범주는 `—`로 표시한다.
- 데이터 축은 0을 포함하는 **선형 자동 범위**다. 로그축·수동 범위·줌은 지원하지 않는다.
  기존 선형 눈금 계산기를 사용하며, 표시 가능한 눈금·라벨 공간은 `prepare`에서 검사한다.

`reorder_categories(&["b", "a"])`는 범주와 모든 값 배열을 함께 옮긴다. 인덱스만 직접 바꾸면
값과 이름이 어긋날 수 있으므로 이 메서드를 권장한다. 선택과 개별 스타일은 ID를 참조하므로
정렬·이름 변경 후에도 같은 항목을 가리킨다. 항목을 삭제하면 해당 선택·개별 설정도 지워야 한다.

## 방향, 스타일과 개별 편집

`direction`은 `Vertical` / `Horizontal`이며 방향을 바꿔도 입력 값이나 범주 배열을 바꿀 필요가
없다. `value_title`과 `category_title`도 각 축의 역할을 따라 이동한다.

스타일은 **차트 → 시리즈 → 개별 막대** 순서로 적용한다. `style: Some(...)`은 전체 스타일을
덮어쓰며 이후 상위 스타일 변경을 상속하지 않는다. `None`으로 돌리면 다시 상속한다.
색은 시리즈의 `color`, 개별 색은 `CategoryBarOverride.color`에서 정한다.

```rust,no_run
use renderer::{CategoricalChart, CategoryBarTarget, CategoryBarOverride,
    CategoryBarStyle, CategoryBarMaterial, CategoryBarLabels, CategoryBarLabelFormat, Color};
# fn edit(chart: &mut CategoricalChart) {
let target = CategoryBarTarget::new("a", "before");
let mut edit = CategoryBarOverride::new(target.clone());
edit.color = Some(Color::from_rgb8(30, 175, 165));
edit.style = Some(CategoryBarStyle {
    material: CategoryBarMaterial::SatinMetal,
    outline: true,
    ..chart.style.clone()
});
edit.labels = Some(CategoryBarLabels::Inside);
edit.label_format = Some(CategoryBarLabelFormat::ValuePercent);
// target당 개별 설정은 하나만 둘 수 있으므로 기존 설정을 지우고 새 설정을 넣는다.
chart.overrides.retain(|o| o.target != target);
chart.overrides.push(edit);
chart.selected = Some(target);
# }
```

| 옵션 | 기본값 | 범위·동작 |
|---|---|---|
| `material` | `Flat` | Flat, Matte, SatinMetal, Enamel, Paper |
| `corner_radius` | 5 | 논리 픽셀 0~32. 막대 두께·길이의 절반 이내로 제한 |
| `outline` | false | 외곽선 켜기 |
| `outline_width` | 1 | 논리 픽셀 0~8. 도형 안쪽으로 그림 |
| `outline_color` | RGB(40,55,75) | RGBA 각 성분 0~1, 알파 허용 |
| `texture_strength` | 0.2 | 0~1, 0이면 무늬 끄기 |
| `texture_scale` | 1 | 0.1~8, 높일수록 무늬가 촘촘함 |
| `gloss` | 0.45 | 0~1, 새틴·에나멜의 반사광 강도 |
| `emphasis_brightness` | 0.08 | 0~0.5, 호버·선택 시 밝기 증가 |
| `group_width` | 0.7 | 범주 칸에서 막대가 차지하는 비율, 0.1~0.95 |
| `bar_gap` | 3 | 묶음 내 막대 사이 논리 픽셀, 0~24 |
| `grid`, `legend` | true | 격자·시리즈 범례 표시 |

묶음 막대는 0 반대편 끝만 둥글게 만든다. 누적 막대는 양수·음수 스택 각각의 마지막
0이 아닌 조각 끝만 둥글게 만들고 내부 경계는 반듯하게 유지한다. 원본 값과 막대의 끝 위치는
재질·외곽선·호버에 따라 바뀌지 않는다. 질감은 셰이더가 생성하며 외부 텍스처 파일을 요구하지 않는다.
범주 색을 비교하기 위한 2D 표현이며 원형 차트의 입체 카메라·돌출 효과는 적용하지 않는다.

## 표기와 글꼴

- `labels`: `None` / `Inside` / `Outside` / `Auto`(기본).
  개별 `labels: None`은 상속, `Some(CategoryBarLabels::None)`은 숨기기다.
- `Auto`: 묶음은 바깥, 누적은 안쪽에 표시하며 공간이 부족한 값 라벨은 생략한다.
  누적 막대의 중간 조각(값이 0이 아니고 스택 끝이 아닌 조각)에 `Outside`가 적용되면 `prepare`와
  `export_rgba`가 `CategoricalError::Invalid`로 실패한다. 차트 전체 `labels`를 `Outside`로 둔 누적 차트도
  중간 조각이 있으면 실패하므로, 누적에서는 `Auto`나 `Inside`를 쓰고 바깥 라벨은 스택 끝 조각에만 개별로 지정한다.
- `label_format`: `Value`(기본) / `Percent` / `ValuePercent` / `CategoryValue`.
  100% 누적도 값 라벨을 자동으로 퍼센트로 바꾸지 않는다. 원하는 표기를 명시한다.
- `label_decimals`: 0~6(기본 1), 불필요한 끝자리 0은 제거한다.
- `value_suffix`: 한 줄 24자까지. 공백을 포함해 `" kg"`처럼 지정한다.
- `font_size`: 논리 픽셀 8~32(기본 14), `font_family`: 기본 `sans-serif`.
- 범주·시리즈 이름과 ID는 제어 문자 없이 80자까지, 제목·축 제목은 160자까지다.

범례는 폭에 맞춰 줄을 나눈다. 내부 값 라벨은 막대 색에
따라 밝은 글자/차트의 `text_color`를 선택한다. 시리즈 범례는 개별 막대 색이 아닌 시리즈 색이다.
한글 등 추가 글꼴은 기존 `register_font_bytes` 경로를 사용한다. `font_family`에 없는 글자는 글자마다
등록 글꼴, 내장 Liberation Sans, 설치된 시스템 글꼴(네이티브 전용) 순으로 찾아 그린다.
글꼴을 등록하면 다음 `prepare`에서 렌더러가 라벨을 새 글꼴로 다시 그리며, 캐시를 직접 비울 필요는 없다.

보이는 값 라벨은 `frame.decorations().hit_test([x, y])`로 찾고
`frame.decorations().drag_by(&mut chart.decorations, hit, dx, dy)`로 하나씩 옮긴다. 옮긴 뒤에는
새 프레임을 준비한다. 이동량은 논리 픽셀 단위의 실수라 소수 단위로 미세하게 조정할 수 있고,
내보내기에도 같은 이동량이 적용된다. 축 눈금 라벨은 묶음으로 움직인다.
[수동 라벨 배치](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#manual-label-placement)를 참고한다.

## 렌더링, 선택과 내보내기

```rust,no_run
use renderer::{CategoricalRenderer, CategoricalChart, RendererDevice, encode_png};
# fn draw(gpu: RendererDevice, chart: &CategoricalChart) -> Result<(), Box<dyn std::error::Error>> {
let mut renderer = CategoricalRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm)?;
// 논리 900×620, 실제 1800×1240에서 도형과 글자를 새로 그린다.
let image = renderer.export_rgba(chart, (900, 620), 2.0)?;
std::fs::write("bars.png", encode_png(&image)?)?;
renderer.end_frame();
# Ok(())
# }
```

실시간 화면에서는 `prepare(&chart, logical_size, scale)`이 반환하는 `Arc<CategoricalFrame>`의
`draw(&mut pass)`를 호출한다. RGBA8/BGRA8의 선형·sRGB 포맷을 지원한다. 출력 타깃은 프레임과
같은 실제 크기·포맷, sample count 1이어야 한다. 최소 논리 크기는 240×200, `scale`은 0.5~4다.
도형의 가장자리는 픽셀당 네 지점으로 AA를 처리한다. `draw`는 타깃 전체 viewport·scissor를
설정하므로 UI 안의 영역에는 별도 텍스처로 그려 배치한다. 축·글자는 세로 2층 아틀라스에
저장하므로 실제 출력 높이의 두 배도 장치의 최대 텍스처 크기 이내여야 한다.

`frame.hit_test([x, y])`에는 차트 내부 **논리 좌표**를 넣는다. 결과는 범주·시리즈 ID다.
`chart.hovered` / `chart.selected`에 쓰고 `prepare`를 다시 호출하면 강조가 반영된다.
`None`으로 해제하며 렌더러가 포인터를 읽거나 애니메이션 타이머를 돌리지 않는다.
`bar_rect(target)`은 보이는 막대의 **물리 픽셀** 경계이며, 둥근 모서리 바깥까지 포함한다.

WASM 생성에는 `CategoricalRenderer::new_async(gpu, format).await`를 사용해
브라우저 파이프라인 컴파일을 기다린다. 출력에는 `export_rgba_async(...).await`를 쓴다. 동기판은 네이티브 전용이다. 결과는
straight-alpha RGBA8이며 PNG는 기존 `encode_png`로 인코딩한다. 내보내기에는 현재 강조 상태가
포함된다. 강조 없는 결과가 필요하면 내보낼 복사본의 `hovered`와 `selected`를 비운다.
브라우저에서는 `crates/web` 예제 래퍼의 `FiggyBoundedChart`로 범주형 막대 문서를 표시·편집·내보낼 수
있다([WASM 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#bounded-chart-host)).

## 캐시와 소유권

모델·논리 크기·배율·글꼴 세대가 같으면 같은 프레임을 재사용한다. 바뀌면 새 불변 프레임을
만들며 이미 받은 프레임은 바뀌지 않는다. 재질·모서리·외곽선·호버·선택 변경은 축·글자
텍스처를 공유한다. 값·배치·색·격자·라벨을 바꾸면 축·글자 텍스처도 다시 만든다. 일반 편집에
`clear_cache`는 필요하지 않다. `shares_annotations_with`는 두 프레임이 축·글자 텍스처를 공유하는지
알려 주며, 이름과 달리 아래 주석 층과는 관계없다.

GPU 예산은 기본 256 MiB다. `set_memory_budget`은 다음 할당을 제한하며 이미 받은 프레임을
버리지 않는다. 검증·배치·예산 오류는 마지막 유효 프레임과 캐시를 보존한다.
`gpu_memory_usage`는 이 렌더러의 살아 있는 자원과 완료 대기 자원을 집계한다. 호스트가
만든 표시용 타깃은 호스트에서 집계해야 한다.

작은 uniform·막대 표는 변경된 프레임마다 할당한다. 호스트는 이전 프레임을 놓고 관련
command buffer를 모두 제출하거나 버린 뒤 `end_frame()`을 호출한다. 네이티브는 장치를 poll해
완료 콜백을 처리한다. `clear_cache`는 내부 참조만 해제하며 외부 `Arc`가 있으면 자원은 유지된다.
원형·Cartesian 렌더러와 원장은 별도이므로 함께 쓰는 호스트는 사용량을 합산한다.

## 실행과 검증

```bash
cargo run -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo run -p figgy-renderer --example categorical_editor --features egui_demo
cargo test -p figgy-model --features serde categorical
cargo test -p figgy-renderer --test categorical_render -- --test-threads=1
cargo test -p figgy-renderer --doc
```

편집기에서 막대를 클릭하거나 목록으로 선택해 색·재질·표기를 바꿀 수 있다. `Whole chart`는
공통 설정을 편집하고 `Reset individual settings`는 선택한 막대의 개별 설정을 지운다.
화면은 창 크기와 DPR에 맞춘 GPU 타깃에 직접 그리며 PNG를 확대해서 표시하지 않는다.

### 회귀 테스트 범위

`categorical_render`는 옵션별 픽셀 변화와 축·글자 텍스처 재사용을 함께 검사한다. 재질별로 실제 적용되는
질감 강도·주기·광택, 외곽선 두께·색·알파, 강조 밝기를 하나씩 바꾸며 이전 값으로 복원했을 때
원래 픽셀로 돌아오는지도 확인한다. Flat에서 질감·광택을 바꿔도 음영이 생기지 않는 경우도 검사한다.
차트→시리즈→막대의 전체 스타일 덮어쓰기와 상속 복원, 개별 색·표기 복원, 소수점·글꼴 등록에
따른 축·글자 텍스처 갱신을 포함한다.

상한 검사는 64범주×8시리즈와 32범주×16시리즈에서 512개 전부의 픽셀과 선택 ID를 확인한다.
범주·시리즈·총 항목 수 상한은 모델에서 각각 검사하며, 렌더러는 초과 입력을 거절한 뒤에도
기존 프레임·캐시·GPU 사용량을 그대로 유지하는지도 확인한다.

`categorical_browser`는 웹 래퍼 없이 renderer crate를 직접 호출하는 WebGPU 테스트다.
범주형 재질·캐시·개별 편집·비동기 PNG 디코딩, 소수 픽셀 누적 경계, 512항목을 검사하며,
원형 차트의 종이 재질과 호버 복원도 확인한다. WebGPU 어댑터가 없으면 실패하고 건너뛰지 않는다.

```bash
# Cargo.lock의 wasm-bindgen 버전과 일치하는 wasm-bindgen-test-runner를 PATH에 둔다.
# 서버가 출력한 주소를 WebGPU가 활성화된 브라우저에서 연다.
NO_HEADLESS=1 CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
  cargo test -p figgy-renderer --test categorical_browser --target wasm32-unknown-unknown
```

CI는 Linux/Vulkan(lavapipe), Windows/DX12(WARP), macOS/Metal(실험적 지원)에서 `categorical_render`를
포함한 네이티브 테스트를 실행한다. `categorical_browser`는 CI에 포함되지 않아 따로 실행하며, GPU 없는
검증 환경에서는 Chromium의 SwiftShader WebGPU를 쓴다. SwiftShader는 이 검증용일 뿐 제품의 기본 설정이 아니다.
실제 GPU 드라이버와 다른 브라우저는 이 검증 범위에 들지 않는다. 기록된 결과는
[`ci/STATUS.md`](https://github.com/shim9610/figgy/blob/master/ci/STATUS.md)에 있다.

## 제목·축·라벨·범례 선택과 이동

데이터 도형의 `hit_test`와 별도로 `frame.decorations()`가 렌더러에서 측정한 히트맵을 제공한다.
기존 `Selectable`·`Draggable` 계약으로 `chart.decorations`를 갱신하고 `prepare`한다.
좌표 단위, 요소별 이동 규칙, 캐시·저장 복원 및 웹 연결은 [공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md)를 참고한다.


### 주석 층

`chart.annotations`의 기준선·구간·텍스트 상자·화살표는 막대 프레임 주위에 별도 패스로 그린다.
`AnnotationAxis::X`는 화면 가로, `Y`는 화면 세로다. `Vertical`이면 `X`가 범주 인덱스(첫 범주 중심이 0,
`-0.5..0.5`가 첫 슬롯), `Y`가 값 축이고 `Horizontal`이면 반대이며, 이때 범주 인덱스는 위에서 아래로
증가한다. 값 축의 좌표는 그려진 자동 범위의 값 단위이며 `PercentStacked`에서는 퍼센트다.
구간은 데이터 아래(underlay)에 그리므로 구간이 있을 때는 `set_background(FrameBackground::HostClear)`로 프레임을 준비하고 호스트가
`chart.background`로 지운다. `export_rgba`는 이를 스스로 처리한다.
`prepare_annotations(chart, size, scale)`의 `BoundedAnnotationFrame`을 `draw_underlay` →
`frame.draw` → `draw_overlay` 순서로 그리고 히트 맵은 `annotations()`로 얻는다. 주석 편집과
`decorations.selected = Annotation(i)` 선택은 막대 프레임과 라벨 래스터를 다시 만들지 않는다.
자세한 계약은 [공통 상호작용 가이드](https://github.com/shim9610/figgy/blob/master/crates/renderer/INTERACTION.md#annotation-layer)를 참고한다.
