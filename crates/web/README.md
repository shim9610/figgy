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

The generated module exports two classes. `FiggyChart` draws Cartesian charts,
including polar plots (`Config.polar`; see the
[polar guide](../renderer/POLAR.md)), categorical bars, box plots and radar
charts. `FiggyBoundedChart` draws radial charts (including donuts).

Earlier releases opened categorical bar, box plot and radar documents through
`FiggyBoundedChart`. `FiggyChart` now opens them:

- `set_categorical_document(json)` reads `CategoricalChart` JSON as category
  bars on a categorical axis.
- `set_boxplot_document(json)` reads `BoxPlotChart` JSON as box plot series
  that read a record and a ragged outlier column.
- `set_radar_document(json)` reads `RadarChart` JSON as radar series on a
  polar chart with a categorical angle axis.

Free functions serve an editor before any chart exists (#314):
`default_config()`, `validate_config(configJson, seriesJson?)` (structured
errors with field paths), `normalize_config(configJson)` and
`convert_categorical_document` / `convert_boxplot_document` /
`convert_radar_document`, which return the converted config and series, the
uploads without their values, the series of each column and notes on every
field the document set that the chart does not carry. Given
`{"chart_area": [x, y, w, h]}` (the chart's chart area) as a second
argument, the conversion is the one the setter installs on that chart, so
a preview and the opened chart agree (#334). Each `set_*_document(json,
options?)` returns the same report JSON from the conversion it installs,
and `{"refuse": ["path", ...] | "any"}` makes it throw `code:
"conversion_loss"` with the refused notes, before anything changes, when
the conversion would drop or change a listed path (or any field). See the
[model API for editors](../renderer/WASM.md#model-api).

The three calls replace the chart atomically: a document that fails to
validate, upload, fit or commit rejects with an `Error` whose `stage` names
the step, and the chart stays as it was. `clear_selection()` ends the
element and annotation selections and any drag, `begin_column_upload(...)`
uploads one column in chunks into a GPU staging buffer and installs it on
`commit(chart)`, and `inspect_streaming_capabilities(config, series)` checks
a candidate document without changing the chart
([edit safety](../renderer/WASM.md#edit-safety)).

`FiggyBoundedChart` keeps its name and its `"radial"` family argument. In this
release its `"categorical"`, `"boxplot"` and `"radar"` families are errors
that name the `FiggyChart` method to use instead, and
`FiggyBoundedChart.supported_families()` returns `["radial"]`
([#315](https://github.com/shim9610/figgy/issues/315); one migration example
per family is in [WASM.md](../renderer/WASM.md#bounded-family-migration)). The `<figgy-chart>` custom
element wraps `FiggyChart` only. Hosts using `FiggyBoundedChart` own their
canvas events and scheduling; see the
[radial chart host API](../renderer/WASM.md#bounded-chart-host).

A page that shows both kinds can keep one GPU device:
`FiggyBoundedChart.create_on(cartesianChart, canvas, "radial", json, width, height)`
(#313) draws the radial chart on a `FiggyChart`'s device and queue, with one
device-loss record and one memory budget. Disposing either chart never
destroys the device the other uses. The radial chart also exports over a
transparent or another background (`export_png_with_background`), keeps a
host's own slice labels while it regenerates the automatic ones after an edit
(`text_generated`, `regenerate_labels`), and puts a moved decoration back
(`reset_decoration_position`).

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

The `<figgy-chart>` element has its own test,
[tests/figgy-chart.test.mjs](tests/figgy-chart.test.mjs). It loads
`figgy-chart.js` against a mock kernel, so it needs Node but no build, no
browser and no GPU. The element is an ES module loaded through `node:vm`, which
needs a flag:

```bash
node --experimental-vm-modules --test crates/web/tests/figgy-chart.test.mjs
```

| Source / guide | Purpose |
|---|---|
| [index.html](index.html) | Interactive histogram, bin/stroke controls and PNG, SVG and PDF output |
| [timestamp-demo.html](timestamp-demo.html) | Timestamp data and axis formatting |
| [figgy-chart.js](figgy-chart.js) | `<figgy-chart>` element: host lifecycle, events and Promise adaptation |
| [src/lib.rs](src/lib.rs), [src/web/](src/web/chart.rs) and [src/radial.rs](src/radial.rs) | wasm-bindgen bridge to the renderer: `FiggyChart` in `src/web/` (one file per concern, `chart.rs` first), `FiggyBoundedChart` (the radial chart host) in `src/radial.rs` |
| [WASM guide](../renderer/WASM.md) (mostly in Korean; the [streaming contract](../renderer/WASM.md#streaming-contract), the document and monotonic-x line parts of section 7, and section 8, the radial chart host with the annotation layer and Cartesian selection helpers, are in English) | Build, lifecycle, streaming, async export and wrapper methods |
| [JSON reference](SCHEMA.md) | The wrapper's JSON representation of the shared model |

## Readiness and selected values

Await `FiggyChart.create(canvas)` before using a raw Cartesian chart. It
waits for its initial empty frame and for no pipeline compile: the renderer
schedules its render pipelines and the browser goes on compiling them after
the chart is on screen. The host calls and awaits nothing for this.
`await chart.prewarm_all()`
remains for a host that wants every pipeline prepared before it goes on.
While a chart's pipelines compile in the background
([#299](https://github.com/shim9610/figgy/issues/299)), a frame draws every
series whose pipelines are ready, with the axes and legend, and leaves the others
out; `frame()` draws again as soon as one of them has compiled, so a chart fills in
without host calls. `chart.frame_complete()` tells whether the last frame drew every
series, and `chart.pipeline_status()` returns `{ screen: { total, ready, compiling,
failed }, all: { … }, frame_complete }`: `screen` counts what the chart on screen
needs, and its `total` changes only with the chart. `await chart.series_ready()` resolves
once a frame draws every series (`first_frame_ready()` also waits for that frame's GPU
work) and `await chart.pipelines_ready()` once every compile has ended; both reject
with an `Error` carrying `label` and `kind` when one failed, and
`chart.pipeline_failures()` lists them as `[{ label, kind, message }]` (`kind`:
`"validation"`, `"internal"`, `"rejected"`, `"device_lost"`, `"cancelled"`).
`<figgy-chart>` has the same methods; its `pipeline_status()` is the last frame's
snapshot, readable while the chart is busy, and it reports `{ scope:
"renderer.pipelines", stage: "screen complete" }` and then `"all complete"` on the
`figgy-init-progress` event. It starts its picker prewarm once the render pipelines
have compiled. Exports and picks always draw the whole chart; they compile what they
need through the browser's asynchronous compiler first (an export its pipelines,
`pick_point` and `pick_data` their exact pick variants), so neither compiles on the
page's thread.
The chart's waits borrow it until they settle. `chart.readiness()` returns a
`FiggyReadiness` that does not ([#315](https://github.com/shim9610/figgy/issues/315)):
its `series_ready()` and `pipelines_ready()` wait for the same compiles without
drawing, its `status()` (`{ screen, all, disposed }`) is readable meanwhile, and the
host can resize, draw or `free()` the chart while it waits; freeing the chart rejects
the wait with an `Error` whose message starts with `"chart disposed"`. See
[Readiness without borrowing the chart](../renderer/WASM.md#readiness-handle).
Annotations can be found by their `id` as well as their index:
`annotation_ids()`, `annotation_index(id)` and `select_annotation_by_id(id)`
([how long an index is valid](../renderer/WASM.md#annotation-ids)). In the generated
`.d.ts`, a method returning JSON text is typed `FiggyJson` (a `string` to
`JSON.parse`), and one returning an object names its interface, such as
`FiggyPipelineStatus` ([return types](../renderer/WASM.md#typescript-return-types)).
A chart starts on the basic shader set, which draws rectangular charts with
linear and logarithmic axes, and moves itself to the full set when its config
first has an axis break, a probability axis or polar coordinates
([#299](https://github.com/shim9610/figgy/issues/299)). A host that knows it
will show such a chart creates it on the full set, so the basic set is not
compiled first and discarded:
`await FiggyChart.create_with_options(canvas, { shaders: "full" })`, or
`<figgy-chart shaders="full">`. The options are `shaders` (`"basic"`, the
default, or `"full"`) and `on_progress` (the callback of
`create_with_progress`); any other option or value is refused with an `Error`
that names it. `chart.shader_set()` and `element.shaders` read the current
set; `chart.require_shader_set("full")` and `element.shaders = "full"` move a
ready chart up early. A chart never goes back to the basic set.
After installing data/configuration, `await chart.first_frame_ready()` submits
that state and waits for GPU completion. For the radial chart, creation
prepares the document, and the host must explicitly await
`view.first_frame_ready()` before presenting the chart as ready. Device loss or
rejected GPU completion must be treated as initialization failure.

A raw Cartesian host can inspect its current typed selection with
`JSON.parse(await chart.selection_details())`. Histogram results include the
selected bin's boundaries, stored value and orientation; a category bar
reports its `category_id`, `category_label`, `value`, `direction` and `mode`.
Points report their
index and coordinates, plus `color_value` / `size_value` when the series has a
point color or size map, and with a size map also `mapped_radius_px` (the
radius `PointSizeMap::radius_px` gives that value, in logical pixels; `null`
for a non-finite value) and `size_hidden` (`true` when the value is not finite
and the draw skips the marker); matrix cells report their indices, coordinates and
cell value; vector series report their `u` and `v` components in place of a
value. The call reads back only the selected values, not whole columns.
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

An editor on the raw `FiggyChart` can ask the renderer, without changing the
document, the selection or the raster
([#312](https://github.com/shim9610/figgy/issues/312)):
`annotation_preview(json)` (where an annotation that is not added yet would be
drawn, in canvas and document pixels), `convert_annotation_anchor(json, kind)`
(the same item with `"Data"` or `"Ratio"` anchors at the same drawn position),
`inspect_document_layout(config_json?, surface_width?, surface_height?)` (the
document area, scale and every panel's frame, data area and layer, for the
installed document or a candidate config, on this canvas or at the physical
surface size given without resizing anything), `panel_at(x, y)` (the panel on top at a point), `data_area_rect()`
(the active layer's exact data area), `legend_entry(series_id)` (one legend row
as the auto legend writes it) and `render_warnings()` (every warning of the
last frame). The free function
`inspect_config_layout(config_json, surface_width, surface_height)`
returns that layout for a candidate without a chart, and equals what a chart
with that surface reports ([#334](https://github.com/shim9610/figgy/issues/334)).
The radial `FiggyBoundedChart` has `annotation_preview(json)` too,
with the same contract and JSON in its document logical pixels
([#334](https://github.com/shim9610/figgy/issues/334)). The
[WASM guide](../renderer/WASM.md#editor-queries-figgychart-312) gives the
result shapes and error codes.

A Feynman diagram's parts are pick and drag targets too: `pick_point`
reports a vertex, a propagator or a label as `{kind: "feynman_vertex" |
"feynman_propagator" | "feynman_label", series_id, index, part?}`, and
`press_feynman(series_id, kind, index, part)` selects it and starts its drag:
`on_move` moves a vertex (the first drag commits the layout), bends a
propagator or moves a label, and `on_release` ends it. The wrapper commits a
diagram's layout as its series is registered; `place_feynman_vertices(series_id)`
commits it again after an edit in place.

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
렌더러에 기능을 더하면 래퍼 메서드도 함께 더한다. WASM으로 컴파일되는 렌더러와 이 래퍼는
서로 다른 계층이며, `FiggyChart`나 `<figgy-chart>`가 모든 Rust 차트 API를 제공하지는 않는다.

직교 좌표 차트와 원형 차트를 한 페이지에 그릴 때는 GPU 장치 하나를 함께 쓸 수 있다.
`FiggyBoundedChart.create_on(cartesianChart, canvas, "radial", json, width, height)`(#313)는
`FiggyChart`의 장치와 큐로 원형 차트를 그리며, 장치 손실 기록과 메모리 예산도 함께 쓴다.
어느 쪽 차트를 해제해도 다른 차트가 쓰는 장치는 파괴되지 않는다. 원형 차트는 투명하거나 다른 색의
배경으로 내보낼 수 있고(`export_png_with_background`), 편집 뒤 자동 라벨을 다시 쓰면서 호스트가 쓴
라벨은 그대로 두며(`text_generated`, `regenerate_labels`), 옮긴 장식을 원래 자리로 돌린다
(`reset_decoration_position`).

Cargo 패키지 `figgy` 0.15.1은 렌더러와 별도로 버전을 매기며 이 저장소에서 빌드한다
(`publish = false`, 배포 대상은 아직 정하지 않았다: [#57](https://github.com/shim9610/figgy/issues/57)).
crates.io의 `figgy` 크레이트와 npm의 `figgy` 패키지는 이 프로젝트와 관계없다.
브라우저 테스트(`tests/*_browser.rs`)는 CI에서 컴파일만 하고 아직 실행하지 않는다.
로컬에서는 `ci/wasm_browser_test.cjs`(Playwright Chromium)로 실행한다.
`<figgy-chart>` 요소 테스트(`tests/figgy-chart.test.mjs`)는 빌드나 브라우저 없이 Node만으로
실행한다: `node --experimental-vm-modules --test crates/web/tests/figgy-chart.test.mjs`.

차트의 파이프라인이 백그라운드에서 컴파일되는 동안([#299](https://github.com/shim9610/figgy/issues/299))
프레임은 파이프라인이 준비된 시리즈와 축·범례를 그리고 나머지는 뺀다. 그 파이프라인의 컴파일이
끝나면 `frame()`이 다시 그리므로 호스트가 호출하지 않아도 차트가 채워진다.
`chart.frame_complete()`는 마지막 프레임이 모든 시리즈를 그렸는지 알려 준다.
`chart.pipeline_status()`는 `{ screen, all, frame_complete }` 객체로 컴파일 현황을 돌려준다.
`screen`은 지금 화면에 필요한 것만 세며, 그 `total`은 차트가 바뀔 때만 바뀐다.
`await chart.series_ready()`는 모든 시리즈가 그려지면, `await chart.pipelines_ready()`는 모든
컴파일이 끝나면 풀린다. 둘 다 컴파일이 실패하면 `label`과 `kind`가 담긴 `Error`로 거부되고,
`chart.pipeline_failures()`가 실패 목록(`[{ label, kind, message }]`)을 돌려준다.
`<figgy-chart>`도 같은 메서드를 갖는다. 요소의 `pipeline_status()`는 마지막 프레임의 스냅샷이라
차트가 바쁠 때도 읽을 수 있고, `figgy-init-progress` 이벤트로 `"screen complete"`와
`"all complete"`를 알린다. 이미지 출력과 피킹은 늘 차트 전체를 그린다.
차트의 대기 메서드는 끝날 때까지 차트를 빌린다. `chart.readiness()`가 돌려주는 `FiggyReadiness`는
차트를 빌리지 않는다([#315](https://github.com/shim9610/figgy/issues/315)). 이 핸들의 `series_ready()`와
`pipelines_ready()`는 같은 컴파일을 기다리되 그리지는 않고, 기다리는 동안에도 `status()`
(`{ screen, all, disposed }`)를 읽거나 차트의 크기를 바꾸고 그리고 `free()`할 수 있다. 차트를 해제하면
기다리던 호출이 `"chart disposed"`로 시작하는 `Error`로 거부된다. 주석은 인덱스뿐 아니라 `id`로도
찾을 수 있다(`annotation_ids()`, `annotation_index(id)`, `select_annotation_by_id(id)`). 생성되는
`.d.ts`에서 JSON 문자열을 돌려주는 메서드는 `FiggyJson`(`JSON.parse`로 읽는 `string`)으로, 객체를
돌려주는 메서드는 `FiggyPipelineStatus` 같은 인터페이스 이름으로 표시된다.
`FiggyBoundedChart.supported_families()`는 `["radial"]`을 돌려준다. 범주형 막대·상자 그림·레이더 차트를
`FiggyChart` 설정 메서드로 옮기는 예제는 [WASM.md](../renderer/WASM.md#bounded-family-migration)에 있다.

차트를 만들기 전에 편집기가 쓸 수 있는 독립 함수가 있다(#314). `default_config()`는 기본 설정을,
`validate_config(configJson, seriesJson?)`는 거부된 필드의 경로가 담긴 검사 결과를,
`normalize_config(configJson)`은 표준 형태로 다시 쓴 설정을 돌려준다.
`convert_categorical_document`, `convert_boxplot_document`, `convert_radar_document`는 레거시 문서를
변환한 설정과 시리즈, 값을 뺀 업로드 목록, 컬럼별 시리즈, 그리고 문서에 있었지만 옮기지 못한
필드의 목록(`notes`)을 돌려준다. 자세한 내용은 [편집기용 모델 API](../renderer/WASM.md#model-api)를 참고한다.

원시 `FiggyChart` 위의 편집기는 문서·선택·래스터를 바꾸지 않고 렌더러에 물어볼 수 있다
([#312](https://github.com/shim9610/figgy/issues/312)). `annotation_preview(json)`는 아직 넣지 않은
주석이 그려질 자리를 캔버스 픽셀과 문서 픽셀로, `convert_annotation_anchor(json, kind)`는 그려지는
위치를 지킨 채 앵커를 `"Data"`나 `"Ratio"`로 바꾼 항목을 돌려준다. `inspect_document_layout(config_json?, surface_width?, surface_height?)`는
설치된 문서나 후보 설정의 문서 영역·배율·패널별 프레임·데이터 영역·레이어를 이 캔버스에서, 또는 캔버스를 바꾸지 않고 주어진 물리 표면 크기에서, `panel_at(x, y)`는
그 점에서 맨 위에 있는 패널을, `data_area_rect()`는 활성 레이어의 정확한 데이터 영역을 돌려준다.
`legend_entry(series_id)`는 자동 범례가 쓰는 한 줄을, `render_warnings()`는 마지막 프레임의 모든
경고를 돌려준다. 원형 차트의 `FiggyBoundedChart`에도 같은 계약과 JSON의 `annotation_preview(json)`가 있으며,
좌표는 그 차트의 문서 논리 픽셀이다([#334](https://github.com/shim9610/figgy/issues/334)).
차트 없이 쓰는 `inspect_config_layout(config_json, surface_width, surface_height)`은 그 표면 크기의 차트와 같은 레이아웃을 돌려준다(#334).
결과 형식과 오류 코드는 [WASM 안내](../renderer/WASM.md#editor-queries-figgychart-312)에 있다.

`set_categorical_document`·`set_boxplot_document`·`set_radar_document`는 차트를 한 번에 바꾼다.
검사·업로드·축 맞춤·반영 중 어느 단계에서 실패하든 `stage`로 그 단계를 알려 주는 `Error`를 던지고
차트는 그대로 둔다. `clear_selection()`은 장식·주석 선택과 진행 중인 끌기를 끝내고,
`begin_column_upload(...)`는 컬럼 하나를 조각으로 나눠 GPU 스테이징 버퍼에 쓴 뒤 `commit(chart)`에서
설치한다. `inspect_streaming_capabilities(config, series)`는 차트를 바꾸지 않고 후보 문서의 스트리밍
가능 여부를 검사한다([edit safety](../renderer/WASM.md#edit-safety)).

차트는 기본 셰이더 세트로 시작하고, 끊긴 축·확률 축·극좌표가 설정에 처음 들어오면 전체 세트로
스스로 올라간다([#299](https://github.com/shim9610/figgy/issues/299)). 처음부터 전체 세트가
필요하면 `FiggyChart.create_with_options(canvas, { shaders: "full" })`이나
`<figgy-chart shaders="full">`로 만든다.

위 명령으로 빌드하고 로컬 서버를 열면 히스토그램과 시간축 데모를 조작할 수 있다.
GitHub README 안에서는 JavaScript와 WASM을 실행할 수 없으므로, 직접 조작하는 데모는
별도 HTTPS 페이지에 올리고 링크로 연결한다. 저장소에는 이 데모를 배포한 실행 페이지 링크가
아직 없다. [figgyplot.com Studio](https://figgyplot.com/)는 렌더러를 사용하는 별도
애플리케이션이며 배포와 UI도 이 저장소의 브라우저용 제품과 별개다.
