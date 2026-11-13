# figgy

[![Rust packages](https://github.com/shim9610/figgy/actions/workflows/ci.yml/badge.svg)](https://github.com/shim9610/figgy/actions/workflows/ci.yml)

**wgpu로 그리는 Rust 차트 라이브러리입니다.** 과학 데이터 그래프와 다양한 스타일의 차트를
만들고, PNG로 출력하거나 직접 만든 애플리케이션에 넣어 편집할 수 있습니다.

**[웹에서 사용해 보기 → figgyplot.com](https://figgyplot.com/)**

[빠른 시작](#빠른-시작) · [문서](#문서) · [English](README.md)

이 문서는 한국어 안내 페이지입니다. 갤러리 이미지, 차트 종류 표, 예제 실행 명령 전체는
[영문 README](README.md)에 있습니다.

- [갤러리](README.md#gallery): 차트를 종류별 썸네일로 모았습니다. 썸네일을 누르면 큰 그림과 설명이, 큰 그림을 누르면 그 그림을 그리는 예제 코드가 열립니다.
- [차트 종류](README.md#chart-types): 차트별 입력 데이터와 안내 문서.
- [예제 목록](crates/renderer/examples/README.md): 그림마다 그리는 함수와 다시 만드는 명령.

## 빠른 시작

**Rust 1.99 이상**과 정상 동작하는 GPU 드라이버가 필요합니다. 저장소의 툴체인은 1.99.0으로
고정했습니다.

애플리케이션의 `Cargo.toml`에 다음 의존성을 추가합니다.

```toml
[dependencies]
renderer = { package = "figgy-renderer", version = "0.18.1" }
```

`version` 줄은 [crates.io](https://crates.io/crates/figgy-renderer)에서 figgy-renderer 0.18.1과
figgy-model 0.12.1을 설치합니다. 로컬에 받은 저장소를 쓰려면 renderer의 `version` 대신
애플리케이션 기준 경로인 `path = "../figgy/crates/renderer"`를 지정합니다.
이전 버전용 코드에서 바꿔야 할 부분은 [변경 이력](CHANGELOG.md)의 마이그레이션 안내에 있습니다.

[최소 선 그래프 예제](crates/renderer/examples/line.rs)를 `src/main.rs`에 복사하고
`cargo run`을 실행하면 창을 열지 않고 `line.png`를 만듭니다.
`figgy-renderer`는 Cargo 패키지 이름이고 Rust 코드에서는 `renderer`로 가져옵니다.
전체 코드는 [영문 README의 빠른 시작](README.md#quick-start)에 있습니다.

예제의 흐름은 다음과 같습니다.

1. `Renderer::headless_blocking()`으로 GPU 장치와 `Renderer`를 만듭니다.
2. `x`와 `y` 컬럼을 등록합니다.
3. 차트 제목·축 범위와 선 스타일을 설정합니다. y축 제목의 `$…$`는 수식으로 그려집니다.
4. `save_blocking`으로 저장합니다. 파일 확장자(`.png`, `.svg`, `.pdf`)가 형식을 정합니다.

픽셀이나 PNG 바이트가 필요하면 `export_rgba_blocking`이나 `export_png_blocking`을 씁니다.
자기 창에 figgy를 넣는 애플리케이션은 `wgpu = "30"` 의존성을 추가하고 `RendererDevice`로 장치를 공유합니다.
실제 데이터를 쓰려면 예제의 벡터를 바꾸거나
[`ColumnSource`](crates/renderer/USAGE.md#columnsource--데이터-어댑터-트레이트)를 구현하면 됩니다.

저장소 루트에서 예제를 바로 실행할 수도 있습니다.

```bash
cargo run --locked -p figgy-renderer --example line
```

한글을 그리려면 시스템에 한글 글꼴이 있거나 `renderer::text_render::register_font_bytes`로
글꼴을 등록해야 합니다. 내장 글꼴(Liberation Sans, Comic Neue)에는 한글이 없고,
WebAssembly에서는 시스템 글꼴을 찾지 않습니다.

기존 프로그램과 GPU 장치를 공유할 때는 **wgpu 30**을 사용합니다.
기본 기능에는 특정 GUI 프레임워크가 필요하지 않습니다.

## 브라우저

Rust 렌더러는 `wasm32-unknown-unknown`으로도 컴파일되며 브라우저에서는 WebGPU를 사용합니다.
브라우저용 제품은 [`crates/web`](crates/web/README.md)(Cargo 패키지 `figgy`, `publish = false`)이며
이 저장소에서 빌드합니다. crates.io의 `figgy` 크레이트와 npm의 `figgy` 패키지는 이 프로젝트와
관계없습니다. [figgyplot.com Studio](https://figgyplot.com/)는 렌더러를 사용하는 별도 웹
애플리케이션입니다.

## 문서

상세 문서는 영어로 쓰며, 아래 문서에는 한국어 절이 함께 있습니다.

| 내용 | 문서 |
|---|---|
| 네이티브 초기화·데이터 연결·화면 통합·출력 | [상세 사용법](crates/renderer/USAGE.md) |
| 축·글자·범례·시리즈·색상표 | [설정 명세](crates/renderer/CONFIG.md) |
| 브라우저 문서 형식 | [JSON 스키마](crates/web/SCHEMA.md) |
| 선택·이동·영역 크기 조정 | [상호작용](crates/renderer/INTERACTION.md) |
| 차트별 안내 | [원형](crates/renderer/RADIAL.md) · [막대](crates/renderer/CATEGORICAL.md) · [박스플롯](crates/renderer/BOXPLOT.md#한국어-사용법) · [레이더](crates/renderer/RADAR.md#한국어-사용법) · [극좌표](crates/renderer/POLAR.md#한국어-사용법) · [히스토그램](crates/renderer/HISTOGRAM.md#한국어-사용법) |
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
내장 글꼴에는 각 OFL·GUST 고지가 적용됩니다.
[Liberation Sans](crates/renderer/fonts/LICENSE-LiberationSans.txt) · [Comic Neue](crates/renderer/fonts/LICENSE-ComicNeue.txt)(SIL OFL 1.1) · [New Computer Modern Math](crates/renderer/fonts/LICENSE-NewCMMath.txt)(GUST Font License, 수식용 글꼴).
