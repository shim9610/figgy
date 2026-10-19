# figgy

**wgpu로 그리는 Rust 차트 라이브러리입니다.** 과학 데이터 그래프와 다양한 스타일의 차트를
만들고, PNG로 출력하거나 직접 만든 애플리케이션에 넣어 편집할 수 있습니다.

**[웹에서 사용해 보기 → figgyplot.com](https://figgyplot.com/)**

[빠른 시작](#빠른-시작) · [예제 실행](#예제-실행) · [문서](#문서) · [English](README.md)

## 갤러리

**이미지를 누르면 해당 차트를 만드는 Rust 코드가 열립니다.** 데이터는 모두 예제용이며,
이미지는 네이티브 렌더러의 실제 출력입니다. GIF는 프레임별 출력 이미지를 이어 붙였습니다.

**선 그래프 · 오차 막대 · 세로 · 가로 히스토그램**

<p>
  <a href="crates/renderer/examples/line.rs"><img src="crates/renderer/assets/gallery-line.png" alt="A sine wave" width="23%" align="top"></a>
  <a href="crates/renderer/examples/error_bars.rs"><img src="crates/renderer/assets/gallery-errorbars.png" alt="Response curves with error bars" width="23%" align="top"></a>
  <a href="crates/renderer/examples/histogram.rs"><img src="crates/renderer/assets/gallery-histogram.png" alt="Vertical histogram" width="23%" align="top"></a>
  <a href="crates/renderer/examples/histogram.rs"><img src="crates/renderer/assets/gallery-histogram-horizontal.png" alt="Horizontal histogram" width="23%" align="top"></a>
</p>

**히트맵과 등고선 · 박스플롯 · 그룹 막대 · 3D 도넛**

<p>
  <a href="crates/renderer/examples/heatmap_contours.rs"><img src="crates/renderer/assets/gallery-contours.png" alt="Heatmap with labeled contours" width="23%" align="top"></a>
  <a href="crates/renderer/examples/boxplot_gallery.rs"><img src="crates/renderer/assets/gallery-boxplot.png" alt="Box plot with outliers" width="23%" align="top"></a>
  <a href="crates/renderer/examples/categorical_gallery.rs"><img src="crates/renderer/assets/gallery-categorical.png" alt="Grouped categorical bars" width="23%" align="top"></a>
  <a href="crates/renderer/examples/radial_gallery.rs"><img src="crates/renderer/assets/gallery-donut.png" alt="Ceramic 3D donut" width="23%" align="top"></a>
</p>

**레이더 · 값 라벨 레이더 · 극좌표 · 나침반**

<p>
  <a href="crates/renderer/examples/radar_gallery.rs"><img src="crates/renderer/assets/gallery-radar.png" alt="Radar chart comparing two releases over six categories on a spider-web grid" width="23%" align="top"></a>
  <a href="crates/renderer/examples/radar_gallery.rs"><img src="crates/renderer/assets/gallery-radar-circle.png" alt="Radar chart on a circular grid with a value label at every vertex" width="23%" align="top"></a>
  <a href="crates/renderer/examples/polar_plot.rs"><img src="crates/renderer/assets/gallery-polar.png" alt="Antenna pattern filled to the centre in polar coordinates" width="23%" align="top"></a>
  <a href="crates/renderer/examples/polar_plot.rs"><img src="crates/renderer/assets/gallery-polar-compass.png" alt="Compass rose: a polar scatter-line, clockwise from the top, with arc segments" width="23%" align="top"></a>
</p>

**움직이는 파형 · 위상 궤적**

<p>
  <a href="crates/renderer/examples/animated_lines.rs"><img src="crates/renderer/assets/gallery-wave.gif" alt="Travelling waves" width="23%" align="top"></a>
  <a href="crates/renderer/examples/animated_lines.rs"><img src="crates/renderer/assets/gallery-orbit.gif" alt="Animated phase portrait" width="23%" align="top"></a>
</p>

[다른 스타일과 이미지 생성 방법](crates/renderer/GALLERY.md).

## 차트 종류

| 차트 | 입력 | 문서 |
|---|---|---|
| 선·산점도·면·계단 시리즈 | `ColumnSource`로 올린 `f64` 컬럼 | [사용법](crates/renderer/USAGE.md) · [시리즈 선언](crates/renderer/CONFIG.md#data_config--시리즈-선언-형식) |
| 오차 막대 | X·Y·오차 컬럼 | [error_bars.rs](crates/renderer/examples/error_bars.rs) |
| 히스토그램 | 구간 경계와 개수 | [히스토그램](crates/renderer/HISTOGRAM.md) |
| 히트맵·등고선 | 축 좌표와 행렬 컬럼 | [heatmap_contours.rs](crates/renderer/examples/heatmap_contours.rs) |
| 벡터장 | 기준점과 (u, v) 성분, 또는 행렬 두 개 | [vector_field.rs](crates/renderer/examples/vector_field.rs) |
| 색·크기 매핑 | 점마다 값 컬럼 하나 | [value_maps.rs](crates/renderer/examples/value_maps.rs) |
| **극좌표 플롯** | (각도, 반지름) 컬럼과 `Config.polar` | [극좌표](crates/renderer/POLAR.md) |
| 파이·도넛 | `RadialChart` 값 | [원형](crates/renderer/RADIAL.md) |
| 범주형 막대 | `CategoricalChart` 범주와 시리즈 | [막대](crates/renderer/CATEGORICAL.md) |
| 박스플롯 | `BoxPlotChart` 요약 통계 | [박스플롯](crates/renderer/BOXPLOT.md) |
| **레이더 차트** | `RadarChart` 범주와 시리즈별 범주당 값 하나 | [레이더](crates/renderer/RADAR.md) |

컬럼 기반 차트는 렌더러·GPU 컬럼 풀·피킹·내보내기 경로를 공유하며 오버레이·서브플롯·스트리밍도
여기에 속합니다. 마지막 네 가지는 소량 데이터 모델과 전용 렌더러를 가진 바운디드 차트입니다.

## 빠른 시작

**Rust 1.99 이상**과 정상 동작하는 GPU 드라이버가 필요합니다. 저장소의 툴체인은 1.99.0으로
고정했습니다. 예제는 renderer **0.17.0**, model **0.11.0**을 기준으로 합니다.
해당 버전이 crates.io에 게시되면 애플리케이션의 `Cargo.toml`에 다음 의존성을 추가합니다.

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.17.0" }
wgpu = "30"
pollster = "1"
```

게시 전 이 소스를 사용하려면 renderer의 `version` 대신 애플리케이션 기준 경로인
`path = "../figgy/crates/renderer"`를 지정합니다.

[최소 선 그래프 예제](crates/renderer/examples/line.rs)를 `src/main.rs`에 복사하고
`cargo run`을 실행하면 창을 열지 않고 `line.png`를 만듭니다.
`figgy-renderer`는 Cargo 패키지 이름이고 Rust 코드에서는 `renderer`로 가져옵니다.
전체 코드는 [영문 README의 빠른 시작](README.md#quick-start)에서도 볼 수 있습니다.

예제의 흐름은 다음과 같습니다.

1. GPU 장치와 `Renderer`를 만듭니다.
2. `x`와 `y` 컬럼을 등록합니다.
3. 차트 제목·축 범위와 선 스타일을 설정합니다.
4. 이미지를 출력하고 PNG 바이트를 파일에 씁니다.

렌더러는 픽셀이나 PNG 바이트를 반환합니다. 저장 위치는 애플리케이션이 정합니다.
실제 데이터를 쓰려면 예제의 벡터를 바꾸거나
[`ColumnSource`](crates/renderer/USAGE.md#columnsource--데이터-어댑터-트레이트)를 구현하면 됩니다.

### 극좌표와 레이더를 몇 줄로

극좌표 플롯은 위 차트에 `polar`를 켠 것입니다. 컬럼은 호스트의 (각도, 반지름) 그대로이고
축은 바깥 고리와 라벨이 붙은 광선이 됩니다.

```rust,ignore
use renderer::{format::LabelFormat, polar::PolarOptions};

config.polar = Some(PolarOptions::default()); // 0°는 오른쪽, 반시계 방향
config.bottom_x.major_spacing = 30.0; // 30°마다 스포크
config.bottom_x.label_style.format = LabelFormat::Degrees;
chart.set_x_range(0.0, 360.0); // 각도 축
chart.set_y_range(0.0, 1.0); // 반지름 축
```

레이더 차트는 전용 렌더러가 그리는 소량 데이터 모델입니다.

```rust,ignore
use renderer::radar::{RadarChart, RadarRenderer, RadarSeries};
use renderer::{Category, Color};

let mut chart = RadarChart::default();
chart.categories = ["속도", "안전", "크기", "문서", "테스트"]
    .iter()
    .enumerate()
    .map(|(i, c)| Category::new(format!("c{i}"), *c))
    .collect();
chart.series = vec![
    RadarSeries::new("v1", "Release 1", vec![6.0, 8.0, 4.0, 5.0, 7.0], Color::from_rgb8(60, 110, 200)),
    RadarSeries::new("v2", "Release 2", vec![8.0, 7.0, 6.0, 8.0, 9.0], Color::from_rgb8(220, 90, 70)),
];
let mut radar = RadarRenderer::new(device, wgpu::TextureFormat::Rgba8Unorm)?;
let frame = radar.prepare(&chart, (720, 520), 1.0)?; // frame.draw(&mut pass)
```

전체 프로그램은 [polar_plot.rs](crates/renderer/examples/polar_plot.rs)와
[radar_gallery.rs](crates/renderer/examples/radar_gallery.rs), 문서는
[극좌표](crates/renderer/POLAR.md)와 [레이더](crates/renderer/RADAR.md)입니다.

## 예제 실행

저장소 루트에서 실행합니다.

```bash
cargo run --locked -p figgy-renderer --example line
cargo run --locked -p figgy-renderer --example error_bars -- target/error-bars
cargo run --locked -p figgy-renderer --example histogram -- target/histogram
cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
```

애니메이션, 편집 가능한 창, 각 차트의 입력 형식은
[예제 목록](crates/renderer/examples/README.md)에 정리했습니다.

## 애플리케이션에 넣기

| 용도 | 예제·문서 |
|---|---|
| 독립 창에서 그리기 | [winit_simple.rs](crates/renderer/examples/winit_simple.rs) |
| egui 화면에 넣기 | [egui_embed.rs](crates/renderer/examples/egui_embed.rs) — `egui_demo` 기능 사용 |
| 원형·막대·박스플롯·레이더 편집 | [네이티브 편집 예제](crates/renderer/examples/README.md#interactive-editors) |
| 극좌표로 선·산점도 그리기 | [극좌표 사용법](crates/renderer/POLAR.md) |
| 선택·이동·영역 크기 조정 연결 | [상호작용 사용법](crates/renderer/INTERACTION.md) |
| GPU 메모리보다 큰 데이터 그리기 | [스트리밍과 상주 전환](crates/renderer/ARCHITECTURE.md#원본-데이터-스트리밍과-상주-전환) |

기존 프로그램과 GPU 장치를 공유할 때는 **wgpu 30**을 사용합니다.
기본 기능에는 특정 GUI 프레임워크가 필요하지 않습니다. 원형·범주형 막대·박스플롯·레이더는
소량 데이터용 모델을 사용하므로 Cartesian 컬럼이나 스트림 API를 거칠 필요가 없습니다.

## WebAssembly 예제

Rust 렌더러는 `wasm32-unknown-unknown`으로도 컴파일되며 브라우저에서는 WebGPU를 사용합니다.
[`crates/web`](crates/web/README.md)은 wasm-bindgen으로 렌더러를 감싸고 캔버스·이벤트·비동기
호출을 연결하는 **브라우저 통합 예제**입니다. Rust 라이브러리의 API와는 별개이며,
이 예제의 `FiggyChart` / `<figgy-chart>`가 렌더러의 모든 차트 종류를 제공하지는 않습니다.

[브라우저 예제 빌드·실행 방법](crates/web/README.md).
[figgyplot.com Studio](https://figgyplot.com/)도 렌더러를 사용하는 별도 애플리케이션입니다.
Studio 자체가 라이브러리에 포함되는 것은 아닙니다.

## 문서

| 내용 | 문서 |
|---|---|
| 네이티브 초기화·데이터 연결·화면 통합·출력 | [상세 사용법](crates/renderer/USAGE.md) |
| 축·글자·범례·시리즈·색상표 | [설정 명세](crates/renderer/CONFIG.md) |
| 원형·범주형 막대·박스플롯·레이더·극좌표·히스토그램 | [원형](crates/renderer/RADIAL.md) · [막대](crates/renderer/CATEGORICAL.md) · [박스플롯](crates/renderer/BOXPLOT.md) · [레이더](crates/renderer/RADAR.md) · [극좌표](crates/renderer/POLAR.md) · [히스토그램](crates/renderer/HISTOGRAM.md) |
| 소유권·프레임 유효성·API 호환성 | [API 계약](crates/renderer/API.md) |
| 메모리·캐시·스트리밍·셰이더 구조 | [기술 문서](crates/renderer/ARCHITECTURE.md) |
| 버전별 변경과 마이그레이션 | [변경 이력](CHANGELOG.md) |
| OS별 검증 결과와 제한 사항 | [CI 결과](ci/STATUS.md) |

Linux/Vulkan과 Windows/DX12에서 네이티브 렌더링·픽셀 검사를 수행합니다.
**macOS/Metal은 아직 실험적 지원입니다.** CI는 소프트웨어 또는 가상 GPU를 사용하며,
이 결과가 모든 실제 GPU 드라이버의 동작을 보장하지는 않습니다.

## 라이선스

코드: [MIT](LICENSE-MIT) 또는 [Apache-2.0](LICENSE-APACHE).
내장 글꼴에는 각 OFL 고지가 적용됩니다.
[Liberation Sans](crates/renderer/fonts/LICENSE-LiberationSans.txt) · [Comic Neue](crates/renderer/fonts/LICENSE-ComicNeue.txt).
