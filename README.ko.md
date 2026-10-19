# figgy

[![Rust packages](https://github.com/shim9610/figgy/actions/workflows/ci.yml/badge.svg)](https://github.com/shim9610/figgy/actions/workflows/ci.yml)

**wgpu로 그리는 Rust 차트 라이브러리입니다.** 과학 데이터 그래프와 다양한 스타일의 차트를
만들고, PNG로 출력하거나 직접 만든 애플리케이션에 넣어 편집할 수 있습니다.

**[웹에서 사용해 보기 → figgyplot.com](https://figgyplot.com/)**

[빠른 시작](#빠른-시작) · [예제 실행](#예제-실행) · [문서](#문서) · [English](README.md)

## 갤러리

**이미지를 누르면 그 이미지를 그리는 예제 파일이 열립니다.**
[예제 목록](crates/renderer/examples/README.md#gallery-images)에 이미지마다 그리는 함수와
다시 만드는 명령이 있습니다. 데이터는 모두 예제용이며,
이미지는 네이티브 렌더러의 실제 출력입니다. GIF는 프레임별 출력 이미지를 이어 붙였습니다.

**선 그래프 · 오차 막대 · 세로 히스토그램 · 가로 히스토그램**

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

**바람장 · 해류 · 색 매핑 점 · 버블 차트**

<p>
  <a href="crates/renderer/examples/vector_field.rs"><img src="crates/renderer/assets/gallery-wind.png" alt="A sea breeze: wind speed as a smooth field with direction arrows on a grid" width="23%" align="top"></a>
  <a href="crates/renderer/examples/vector_field.rs"><img src="crates/renderer/assets/gallery-currents.png" alt="Surface currents at scattered drifters, arrow length and colour by speed" width="23%" align="top"></a>
  <a href="crates/renderer/examples/value_maps.rs"><img src="crates/renderer/assets/gallery-colormap.png" alt="Particles along two spiral arms coloured by age through the colourbar" width="23%" align="top"></a>
  <a href="crates/renderer/examples/value_maps.rs"><img src="crates/renderer/assets/gallery-bubbles.png" alt="Bubble chart of income and life expectancy for three regions, area by population" width="23%" align="top"></a>
</p>

**예측 구간 · 계단 스케줄 · 쌍둥이 축 · 소형 다중 차트**

<p>
  <a href="crates/renderer/examples/area_step.rs"><img src="crates/renderer/assets/gallery-forecast.png" alt="A demand forecast: the history solid, the forecast dashed inside 80 and 95 percent bands" width="23%" align="top"></a>
  <a href="crates/renderer/examples/area_step.rs"><img src="crates/renderer/assets/gallery-thermostat.png" alt="A thermostat schedule as a filled step line under the measured room temperature" width="23%" align="top"></a>
  <a href="crates/renderer/examples/overlay_axes.rs"><img src="crates/renderer/assets/gallery-twin-axes.png" alt="A battery discharge: voltage on the left axis, current on a right overlay axis" width="23%" align="top"></a>
  <a href="crates/renderer/examples/subplots.rs"><img src="crates/renderer/assets/gallery-subplots.png" alt="Four sensors as a 2 by 2 grid of subplots sharing the hour axis" width="23%" align="top"></a>
</p>

**움직이는 파형 · 위상 궤적**

<p>
  <a href="crates/renderer/examples/animated_lines.rs"><img src="crates/renderer/assets/gallery-wave.gif" alt="Travelling waves" width="23%" align="top"></a>
  <a href="crates/renderer/examples/animated_lines.rs"><img src="crates/renderer/assets/gallery-orbit.gif" alt="Animated phase portrait" width="23%" align="top"></a>
</p>

**대시보드 · 스케치 · 팝아트**

<p>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-observability.png" alt="Dark service dashboard with mint and lavender traffic curves" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-revenue.png" alt="Light revenue dashboard with horizontal rounded stacks" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-editorial.png" alt="Warm paper-colored sketch chart with hand-drawn lines and lettering" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-pop-art.png" alt="Bright toon-shaded pie with dark outlines and shallow extrusion" width="23%" align="top"></a>
</p>

**펄 · 지형**

<p>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-portfolio.png" alt="Oblique pearl-material donut on a dark background" width="23%" align="top"></a>
  <a href="crates/renderer/examples/gallery_concepts.rs"><img src="crates/renderer/assets/concept-terrain.png" alt="Synthetic terrain with a forest-to-sand color ramp and fine isolines" width="23%" align="top"></a>
</p>

**그리기 스타일: 정밀 · 손그림 스케치 · 은하수 · 별자리**

<p>
  <a href="crates/renderer/examples/precise_sketch_preview.rs"><img src="crates/renderer/assets/style-growth-response-precise.png" alt="Precise growth curves with clean lines and markers" width="23%" align="top"></a>
  <a href="crates/renderer/examples/precise_sketch_preview.rs"><img src="crates/renderer/assets/style-growth-response-sketch.png" alt="Sketch growth curves with hand-drawn lines, axes and lettering" width="23%" align="top"></a>
  <a href="crates/renderer/examples/milkyway_line_preview.rs"><img src="crates/renderer/assets/style-growth-response-milkyway.png" alt="Milkyway line-only growth curves with stars and nebula" width="23%" align="top"></a>
  <a href="crates/renderer/examples/constellation_preview.rs"><img src="crates/renderer/assets/style-growth-response-constellation.png" alt="Constellation growth curves with stars at the source samples" width="23%" align="top"></a>
</p>

은하수 미리보기에는 두 곡선을 선 시리즈로 그려 별과 성운 띠만 나타납니다. 은하수 스타일은 산점도도
고리 달린 행성으로, 오차 막대도 제트로 그립니다. 별자리 스타일은 `ScatterLine` 시리즈만 그리며
원본 표본 위치에 별을 놓습니다.
[편집 가능한 시각 레시피 여섯 가지](crates/renderer/GALLERY.md#visual-concepts)는 같은 차트 API에서
팔레트·축·재질·라벨을 어떻게 바꾸는지 보여 줍니다.

[다른 스타일과 이미지 생성 방법](crates/renderer/GALLERY.md).

## 차트 종류

| 차트 | 입력 | 문서 |
|---|---|---|
| 선·산점도 | `ColumnSource`로 올린 `f64` 컬럼 | [사용법](crates/renderer/USAGE.md) · [시리즈 선언](crates/renderer/CONFIG.md#data_config--시리즈-선언-형식) |
| 면 채움·계단선 | 선의 `fill`(기준선 또는 다른 시리즈까지)과 `interpolation` | [면 채움과 계단선](crates/renderer/CONFIG.md#면-채움과-계단선) · [area_step.rs](crates/renderer/examples/area_step.rs) |
| 오차 막대 | X·Y·오차 컬럼 | [error_bars.rs](crates/renderer/examples/error_bars.rs) |
| 히스토그램 | 구간 경계와 개수 | [히스토그램](crates/renderer/HISTOGRAM.md#한국어-사용법) |
| 히트맵·등고선 | 축 좌표와 행렬 컬럼 | [heatmap_contours.rs](crates/renderer/examples/heatmap_contours.rs) |
| 벡터장(화살표) | 기준점과 (u, v) 컬럼, 또는 격자 위 행렬 두 개 | [벡터장](crates/renderer/CONFIG.md#벡터장) · [vector_field.rs](crates/renderer/examples/vector_field.rs) |
| 색·크기 매핑(버블) | 점마다 값 하나가 든 컬럼(색·크기에 각각 하나)과 색상 막대 또는 크기 범위 | [점 색·크기 매핑](crates/renderer/CONFIG.md#점-색과-크기-매핑) · [value_maps.rs](crates/renderer/examples/value_maps.rs) |
| 보조 Y축·서브플롯·주석 | 오른쪽 보조 Y축, 한 문서 안의 차트 격자, 기준선·구간·텍스트·화살표 | [오버레이](crates/renderer/INTERACTION.md#overlay-layers) · [서브플롯](crates/renderer/INTERACTION.md#subplots) · [주석](crates/renderer/INTERACTION.md#annotation-layer) |
| 극좌표 플롯 | (각도, 반지름) 컬럼과 `Config.polar` | [극좌표](crates/renderer/POLAR.md#한국어-사용법) |
| 파이·도넛 | `RadialChart` 값 | [원형](crates/renderer/RADIAL.md) |
| 범주형 막대 | `CategoricalChart` 범주와 시리즈 | [막대](crates/renderer/CATEGORICAL.md) |
| 박스플롯 | `BoxPlotChart` 요약 통계 | [박스플롯](crates/renderer/BOXPLOT.md#한국어-사용법) |
| 레이더 차트 | `RadarChart` 범주와 시리즈별 범주당 값 하나 | [레이더](crates/renderer/RADAR.md#한국어-사용법) |

컬럼 기반 차트는 렌더러·GPU 컬럼 풀·피킹·내보내기 경로를 공유하며, 오버레이와 서브플롯은 이
차트에서 씁니다. 단, 극좌표 차트에는 보조 Y축(오버레이)을 둘 수 없습니다. GPU 메모리보다 큰
데이터를 나눠 그리는 스트리밍은 선·산점도·오차 막대·히스토그램·히트맵을 지원합니다. 극좌표·면 채움·계단선·벡터장·점 색·크기 매핑·등고선 시리즈나 오버레이·서브플롯이 있는
문서에 스트리밍 소스를 쓰면 오류를 반환합니다. 파이·도넛·범주형 막대·박스플롯·레이더는 각자 소량 데이터
모델과 전용 렌더러를 쓰며 컬럼이나 스트리밍 API를 거치지 않습니다. 주석 레이어는 두 종류의 차트에서
모두 쓸 수 있습니다.

## 빠른 시작

**Rust 1.99 이상**과 정상 동작하는 GPU 드라이버가 필요합니다. 저장소의 툴체인은 1.99.0으로
고정했습니다.

애플리케이션의 `Cargo.toml`에 다음 의존성을 추가합니다.

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.18.0" }
wgpu = "30"
pollster = "1"
```

`version` 줄은 [crates.io](https://crates.io/crates/figgy-renderer)에서 figgy-renderer 0.18.0과
figgy-model 0.12.0을 설치합니다. 로컬에 받은 저장소를 쓰려면 renderer의 `version` 대신
애플리케이션 기준 경로인 `path = "../figgy/crates/renderer"`를 지정합니다.
0.17.0용 코드에서 바꿔야 할 부분은 [마이그레이션 안내](CHANGELOG.md#migrating-from-0170)에 있습니다.

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

극좌표 플롯은 위 선 그래프의 `chart`에 `polar`를 켠 것입니다. 컬럼에는 (각도, 반지름) 값을
그대로 넣습니다. 각도 축(`bottom_x`)은 바깥 원이 되고, 반지름 축(`left_y`)은 라벨이 붙은 반직선이
됩니다. `set_y_range`는 반지름 축 범위를 정하고, `set_x_range`는 그릴 각도 눈금·스포크·라벨만
정합니다. 원은 항상 `zero_angle_degrees`에서 시작하는 한 바퀴(360° 또는 2π)이므로 0~180 범위를
줘도 전체 원으로 늘어나지 않으며, 원의 일부만 보이려면 `PolarOptions`의 `sector`를 씁니다.
두 범위 함수는 눈금 간격과 라벨 형식도 다시 정하므로 `config_mut`로 눈금을 바꾸기 전에 호출합니다.

```rust,ignore
use renderer::{format::LabelFormat, polar::PolarOptions};

chart.set_x_range(0.0, 360.0); // 그릴 각도 눈금과 스포크
chart.set_y_range(0.0, 1.0); // 반지름 축
let config = chart.config_mut();
config.polar = Some(PolarOptions::default()); // 0°는 오른쪽, 반시계 방향
config.bottom_x.major_spacing = 30.0; // 30°마다 스포크
config.bottom_x.label_style.format = LabelFormat::Degrees;
```

레이더 차트는 `RadarChart` 모델을 만들고 전용 `RadarRenderer`로 그립니다.

```rust,ignore
use renderer::radar::{RadarChart, RadarRenderer, RadarSeries};
use renderer::{Category, Color, RendererDevice, encode_png};
use std::sync::Arc;

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
// device와 queue는 빠른 시작처럼 request_device로 얻습니다.
let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
let mut radar = RadarRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm)?;
let image = radar.export_rgba(&chart, (720, 520), 1.0)?;
std::fs::write("radar.png", encode_png(&image)?)?;
// 창에 그릴 때는 radar.prepare(&chart, (720, 520), 1.0)?로 프레임을 만들고
// 호스트의 렌더 패스에서 frame.draw(&mut pass)를 호출합니다.
```

한글 범주 이름을 그리려면 시스템에 한글 글꼴이 있거나
`renderer::text_render::register_font_bytes`로 글꼴을 등록해야 합니다. 내장 글꼴(Liberation Sans,
Comic Neue)에는 한글이 없고, WebAssembly에서는 시스템 글꼴을 찾지 않습니다.

전체 프로그램은 [polar_plot.rs](crates/renderer/examples/polar_plot.rs)와
[radar_gallery.rs](crates/renderer/examples/radar_gallery.rs), 문서는
[극좌표](crates/renderer/POLAR.md#한국어-사용법)와 [레이더](crates/renderer/RADAR.md#한국어-사용법)입니다.

## 예제 실행

저장소 루트에서 실행합니다.

```bash
cargo run --locked -p figgy-renderer --example line
cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts
cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field
cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
cargo run --locked -p figgy-renderer --example area_step -- target/area-step
cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes
cargo run --locked -p figgy-renderer --example subplots -- target/subplots
cargo run --locked -p figgy-renderer --example error_bars -- target/error-bars
cargo run --locked -p figgy-renderer --example histogram -- target/histogram
cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
```

갤러리 그림마다 그리는 함수와 실행 명령, 애니메이션, 편집 가능한 창은
[예제 목록](crates/renderer/examples/README.md)에 정리했습니다.

## 애플리케이션에 넣기

| 용도 | 예제·문서 |
|---|---|
| 독립 창에서 그리기 | [winit_simple.rs](crates/renderer/examples/winit_simple.rs) |
| egui 화면에 넣기 | [egui_embed.rs](crates/renderer/examples/egui_embed.rs) — `egui_demo` 기능 사용 |
| 원형·막대·박스플롯·레이더 편집 | [네이티브 편집 예제](crates/renderer/examples/README.md#interactive-examples) |
| 극좌표로 선·산점도 그리기 | [극좌표 사용법](crates/renderer/POLAR.md#한국어-사용법) |
| 선택·이동·영역 크기 조정 연결 | [상호작용 사용법](crates/renderer/INTERACTION.md) |
| GPU 메모리보다 큰 데이터 그리기 | [스트리밍과 상주 전환](crates/renderer/ARCHITECTURE.md#원본-데이터-스트리밍과-상주-전환) |

기존 프로그램과 GPU 장치를 공유할 때는 **wgpu 30**을 사용합니다.
기본 기능에는 특정 GUI 프레임워크가 필요하지 않습니다.

## WebAssembly 예제

Rust 렌더러는 `wasm32-unknown-unknown`으로도 컴파일되며 브라우저에서는 WebGPU를 사용합니다.
[`crates/web`](crates/web/README.md)은 wasm-bindgen으로 렌더러를 감싸고 캔버스·이벤트·비동기
호출을 연결하는 브라우저 통합 예제이며, Rust 라이브러리의 API와는 별개입니다.
직교 차트(극좌표·오버레이·서브플롯 포함)는 `FiggyChart`로, 파이·도넛·범주형 막대·박스플롯·레이더는
`FiggyBoundedChart`로 그립니다. `<figgy-chart>` 요소는 `FiggyChart`만 감싸므로 `FiggyBoundedChart`를
쓰는 호스트는 캔버스 이벤트와 프레임 예약을 직접 맡습니다.

[브라우저 예제 빌드·실행 방법](crates/web/README.md).
[figgyplot.com Studio](https://figgyplot.com/)는 렌더러를 사용하는 별도 웹 애플리케이션이며
`figgy-renderer` 크레이트나 브라우저 예제에 포함되지 않습니다.

## 문서

| 내용 | 문서 |
|---|---|
| 네이티브 초기화·데이터 연결·화면 통합·출력 | [상세 사용법](crates/renderer/USAGE.md) |
| 축·글자·범례·시리즈·색상표 | [설정 명세](crates/renderer/CONFIG.md) |
| 원형·범주형 막대·박스플롯·레이더·극좌표·히스토그램 | [원형](crates/renderer/RADIAL.md) · [막대](crates/renderer/CATEGORICAL.md) · [박스플롯](crates/renderer/BOXPLOT.md#한국어-사용법) · [레이더](crates/renderer/RADAR.md#한국어-사용법) · [극좌표](crates/renderer/POLAR.md#한국어-사용법) · [히스토그램](crates/renderer/HISTOGRAM.md#한국어-사용법) |
| 소유권·프레임 유효성·API 호환성 | [API 계약](crates/renderer/API.md) |
| 메모리·캐시·스트리밍·셰이더 구조 | [기술 문서](crates/renderer/ARCHITECTURE.md) |
| 버전별 변경과 마이그레이션 | [변경 이력](CHANGELOG.md) |
| OS별 검증 결과와 제한 사항 | [CI 결과](ci/STATUS.md) |

Linux/Vulkan과 Windows/DX12에서 네이티브 렌더링·픽셀 검사를 수행합니다.
**macOS/Metal은 아직 실험적 지원입니다.** CI는 소프트웨어 또는 가상 GPU를 사용하며,
이 결과가 모든 실제 GPU 드라이버의 동작을 보장하지는 않습니다.

## 기여와 보안

버그 보고, 개발 검사, 풀 리퀘스트 절차는 [CONTRIBUTING.md](CONTRIBUTING.md)에 있습니다.
보안 취약점으로 의심되는 문제는 [SECURITY.md](SECURITY.md)에 따라 비공개로 알려 주세요.

## 라이선스

코드: [MIT](LICENSE-MIT) 또는 [Apache-2.0](LICENSE-APACHE).
내장 글꼴에는 각 OFL 고지가 적용됩니다.
[Liberation Sans](crates/renderer/fonts/LICENSE-LiberationSans.txt) · [Comic Neue](crates/renderer/fonts/LICENSE-ComicNeue.txt).
