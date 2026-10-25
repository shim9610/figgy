# figgy for the browser

`crates/web` is figgy's browser product: the wasm-bindgen wrapper (`FiggyChart`,
`FiggyBoundedChart`) and the `<figgy-chart>` custom element, which owns a
canvas, forwards pointer events, schedules frames and adapts async calls. It is
maintained with the renderer: every renderer feature ships its wrapper methods.

The library packages are `figgy-renderer` 0.18.1 and `figgy-model` 0.12.1. The
browser product is the Cargo package `figgy` 0.15.1, which has its own version
and is named for its generated JS/WASM filenames. It is built from this
repository against the renderer and model sources in the checkout through path
dependencies (`publish = false`; the publication target is open,
[#57](https://github.com/shim9610/figgy/issues/57)). The `figgy` crate on crates.io and the
`figgy` package on npm are unrelated projects. An application that reuses this
wrapper in its own crate depends on the published renderer with the `serde`
feature, which the wrapper's JSON methods need:

```toml
renderer = { package = "figgy-renderer", version = "0.18.1", features = ["serde"] }
```

The generated module exports `FiggyChart` for Cartesian charts, including polar
plots (`Config.polar`; see the [polar guide](../renderer/POLAR.md)) and
categorical bars, box plots and radar charts, and `FiggyBoundedChart` for
radial charts (including donuts). `FiggyChart.set_categorical_document(json)`
opens a categorical bar document (`CategoricalChart` JSON, which
`FiggyBoundedChart` took as the `"categorical"` family before) as category bars
on a categorical axis, and `FiggyChart.set_boxplot_document(json)` a box plot
document (`BoxPlotChart` JSON, the former `"boxplot"` family) as box plot
series reading a record and a ragged outlier column, and
`FiggyChart.set_radar_document(json)` a radar document (`RadarChart` JSON, the
former `"radar"` family) as radar series on a polar chart with a categorical
angle axis. The `<figgy-chart>` custom element wraps `FiggyChart` only. Hosts
using `FiggyBoundedChart` own their canvas events and scheduling; see the
[bounded host API](../renderer/WASM.md#bounded-chart-host).

## Build and run

From the repository root, with Rust 1.99.0 and Node installed:

```bash
rustup target add wasm32-unknown-unknown
npx --yes wasm-pack@0.15.0 build crates/web --release --target web --locked
npx --yes serve crates/web -l 8142
```

Open <http://localhost:8142/> for the interactive histogram, or
<http://localhost:8142/timestamp-demo.html> for timestamp axes. Use a browser
with WebGPU enabled. Localhost is a secure context; remote hosting needs HTTPS.
Opening the HTML directly as a `file:` URL is not the supported flow.

The pages and [figgy-chart.js](figgy-chart.js) load the generated `pkg/` files.
Keep the JavaScript and WASM from the same build together. `pkg/` is generated,
not checked into the source snapshot.

The browser tests (`tests/*_browser.rs`) compile in CI but do not run there yet.
Run them locally with [`ci/wasm_browser_test.cjs`](../../ci/wasm_browser_test.cjs)
(Playwright Chromium), as the [CI guide](../../ci/README.md) describes.

| Source / guide | Purpose |
|---|---|
| [index.html](index.html) | Interactive histogram, bin/stroke controls and PNG output |
| [timestamp-demo.html](timestamp-demo.html) | Timestamp data and axis formatting |
| [figgy-chart.js](figgy-chart.js) | `<figgy-chart>` element: host lifecycle, events and Promise adaptation |
| [src/lib.rs](src/lib.rs), [src/web/](src/web/chart.rs) and [src/bounded.rs](src/bounded.rs) | wasm-bindgen bridge to the renderer: `FiggyChart` in `src/web/` (one file per concern, `chart.rs` first), `FiggyBoundedChart` in `src/bounded.rs` |
| [WASM guide](../renderer/WASM.md) (in Korean; the [streaming contract](../renderer/WASM.md#streaming-contract) and section 8, the bounded chart host with the annotation layer and Cartesian selection helpers, are in English) | Build, lifecycle, streaming, async export and wrapper methods |
| [JSON reference](SCHEMA.md) | The wrapper's JSON representation of the shared model |

## Readiness and selected values

Await `FiggyChart.create(canvas)` before using a raw Cartesian chart. It
prepares its rendering pipelines and waits for its initial empty frame.
After installing data/configuration, `await chart.first_frame_ready()` submits
that state and waits for GPU completion. For bounded charts, creation prepares
the requested family, and the host must explicitly await
`view.first_frame_ready()` before presenting the chart as ready. Device loss or
rejected GPU completion must be treated as initialization failure.

A raw Cartesian host can inspect its current typed selection with
`JSON.parse(await chart.selection_details())`. Histogram results include the
selected bin's boundaries, stored value and orientation; a category bar
reports its `category_id`, `category_label`, `value`, `direction` and `mode`.
Points report their
index and coordinates, plus `color_value` / `size_value` when the series has a
point color or size map; matrix cells report their indices, coordinates and
cell value; vector series report their `u` and `v` components in place of a
value. Only the selected GPU samples are read back.
`chart.step_selection(dx, dy)` moves the selection through resident data:
points (point-vector arrows included), histogram bins, category bars (along
the categories and across the series of the bar group) and matrix cells
(grid-vector cells included). These two methods are raw `FiggyChart` exports,
not custom-element forwarders. See the
[selection contract](SCHEMA.md#resident-selection-details).

The value labels of category bars, box plots and radar series are pick and
drag targets: `pick_point` and `pick_data` report a label under the pointer
with its `label_id` (`"series:<series_id>:label:<index>"`), and
`press_value_label(label_id)` selects it and starts its drag, which
`on_move` continues (writing the series' `labels.offsets`) and `on_release`
ends. A selected label reports its category, string and offset in
`selection_details()`; `step_selection` does not step it.

Serialize asynchronous operations and keep rendering, pointer edits, resize
and disposal behind the same gate until the operation finishes. This includes
selected-value reads; wasm-bindgen does not queue competing calls. The
[async host guide](../renderer/WASM.md#39-async-메서드와-객체-잠금-필독)
(in Korean) describes the lifecycle contract.

## Linking a live demo from GitHub

GitHub repository READMEs display images and GIFs but do not execute JavaScript,
WASM or embedded iframes. An interactive chart needs its own page. The files
above can be hosted on an HTTPS static site such as GitHub Pages after building
the matching `pkg/`. Do not publish an empty `pkg/` or link to an unbuilt page.
A link to a repository HTML file opens its source on GitHub, not a running chart.

The repository links no hosted build of these pages.
[figgyplot.com Studio](https://figgyplot.com/) is a separate application built
on the renderer, with its own deployment and UI.

## 한국어 안내

이 디렉터리는 figgy의 브라우저용 제품이다. wasm-bindgen 래퍼(`FiggyChart`, `FiggyBoundedChart`)와
`<figgy-chart>` 사용자 정의 요소가 캔버스·이벤트·비동기 호출을 연결한다. 렌더러와 함께 관리하므로
렌더러에 기능이 추가되면 래퍼 메서드도 함께 추가된다. WASM으로 컴파일되는 렌더러와 이 래퍼는
서로 다른 계층이며, `FiggyChart`나 `<figgy-chart>`가 모든 Rust 차트 API를 제공하는 것은 아니다.

Cargo 패키지 `figgy` 0.15.1은 렌더러와 별도로 버전을 매기며 이 저장소에서 빌드한다
(`publish = false`, 배포 대상은 아직 정하지 않았다: [#57](https://github.com/shim9610/figgy/issues/57)).
crates.io의 `figgy` 크레이트와 npm의 `figgy` 패키지는 이 프로젝트와 관계없다.
브라우저 테스트(`tests/*_browser.rs`)는 CI에서 컴파일만 하고 아직 실행하지 않는다.
로컬에서는 `ci/wasm_browser_test.cjs`(Playwright Chromium)로 실행한다.

위 명령으로 빌드하고 로컬 서버를 열면 히스토그램과 시간축 데모를 조작할 수 있다.
GitHub README 안에서는 JavaScript와 WASM을 실행할 수 없으므로, 직접 조작하는 데모는
별도 HTTPS 페이지에 올리고 링크로 연결한다. 저장소에는 이 데모를 배포한 실행 페이지 링크가
아직 없다. [figgyplot.com Studio](https://figgyplot.com/)는 렌더러를 사용하는 별도
애플리케이션이며 배포와 UI도 이 저장소의 브라우저용 제품과 별개다.
