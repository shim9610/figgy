# Chart examples

Run these commands from the workspace root with the pinned Rust toolchain.
Headless examples need a working GPU adapter, but no window or browser.
They use normal device selection; a software driver is an environment choice
for machines without a GPU.

## Charts and input data

| Example | Input / output | Run |
|---|---|---|
| [line.rs](line.rs) | Two `f64` columns → `line.png`; complete minimal setup | `cargo run --locked -p figgy-renderer --example line` |
| [overlay_axes.rs](overlay_axes.rs) | Temperature on the base layer, pressure on an overlay layer with its own right Y axis → one document PNG | `cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes.png` |
| [subplots.rs](subplots.rs) | A 2×2 grid of charts with their own axes, one X axis linked, one twin Y axis → one document PNG | `cargo run --locked -p figgy-renderer --example subplots -- target/subplots.png` |
| [gallery_concepts.rs](gallery_concepts.rs) | Six visual recipes: dark monitoring, light analytics, editorial sketch, pop-art pie, pearl donut and topographic color | `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts` |
| [area_step.rs](area_step.rs) | Shared x column → area under a line, a band between two lines and the three step modes | `cargo run --locked -p figgy-renderer --example area_step -- target/area-step` |
| [value_maps.rs](value_maps.rs) | X/Y plus two value columns → points coloured through the colourbar, and a bubble chart whose marker area follows a column | `cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps` |
| [vector_field.rs](vector_field.rs) | Anchors plus (u, v) columns → arrows over a stream function, and two matrices over a heatmap's grid → a colour-mapped quiver | `cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field` |
| [polar_plot.rs](polar_plot.rs) | (angle, radius) columns with `Config.polar` → an antenna pattern filled to the centre, a compass rose with arc segments, a dashed spiral and a half-circle sector in radians | `cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot` |
| [error_bars.rs](error_bars.rs) | X/Y/error columns → lines, markers and symmetric error bars | `cargo run --locked -p figgy-renderer --example error_bars -- target/error-bars` |
| [histogram.rs](histogram.rs) | N+1 bin edges and N counts → vertical and horizontal histograms | `cargo run --locked -p figgy-renderer --example histogram -- target/histogram` |
| [heatmap_contours.rs](heatmap_contours.rs) | Coordinate axes and matrix columns → heatmap, labeled contours and colorbar | `cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours` |
| [animated_lines.rs](animated_lines.rs) | Updated columns → travelling waves and a moving point/trail | `cargo run --locked -p figgy-renderer --example animated_lines -- target/animation 48` |
| [radial_gallery.rs](radial_gallery.rs) | Named values and colors → pie/donut, materials, rounded edges and lift | `cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery` |
| [categorical_gallery.rs](categorical_gallery.rs) | Categories and optional values per series → grouped/stacked/percent bars | `cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery` |
| [boxplot_gallery.rs](boxplot_gallery.rs) | Supplied five-number summaries, optional outliers/mean/CI → boxes | `cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery` |
| [radar_gallery.rs](radar_gallery.rs) | Categories and one value per category per series → spider-web or circular grids, markers, value labels and selection emphasis | `cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery` |
| [milkyway_line_preview.rs](milkyway_line_preview.rs) | Two line series → stars and nebula along the curves | `cargo run --locked -p figgy-renderer --example milkyway_line_preview -- target/milkyway.png` |
| [constellation_preview.rs](constellation_preview.rs) | Two sparse `ScatterLine` series → stars at the source samples | `cargo run --locked -p figgy-renderer --example constellation_preview -- target/constellation.png` |
| [precise_sketch_preview.rs](precise_sketch_preview.rs) | The same sparse growth curves → Precise and hand-drawn Sketch comparisons | `cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch` |

Box plots consume statistics supplied by the caller; they do not infer quartiles
from raw observations. Radial, categorical, boxplot and radar models are bounded
inputs, separate from the Cartesian column and replayable streaming contracts.
Animation examples write PNG frames; see [GIF encoding](../GALLERY.md#encode-the-gifs).

[support/gallery.rs](support/gallery.rs) shares only device setup, presentation
defaults and PNG checks. Each chart's data, series and settings are in its linked
example. [readme_gallery.rs](readme_gallery.rs) calls the same Cartesian examples
to reproduce the gallery in one run.

## Interactive editors

These open native windows. They demonstrate host event handling as well as drawing.

```bash
cargo run --locked -p figgy-renderer --example winit_simple
cargo run --locked -p figgy-renderer --features egui_demo --example egui_embed
cargo run --locked -p figgy-renderer --features egui_demo --example radial_editor
cargo run --locked -p figgy-renderer --features egui_demo --example categorical_editor
cargo run --locked -p figgy-renderer --features egui_demo --example boxplot_editor
cargo run --locked -p figgy-renderer --features egui_demo --example radar_editor
```

The bounded editors show decoration selection/dragging and `Edit plot area` with
eight resize handles. [Interaction contracts](../INTERACTION.md) explain logical
coordinates, hit priority, incremental deltas, rejected edits and export state.
The [streaming capacity demo](streaming_capacity_demo.rs) and [SSOT lab](ssot_lab.rs)
cover larger Cartesian datasets; both require `egui_demo`.

The [browser integration example](https://github.com/shim9610/figgy/blob/master/crates/web/README.md) is a separate WASM consumer.
It demonstrates a host bridge and does not expose every native chart API.

## 한국어 안내

표의 링크에는 데이터와 차트 설정이 들어 있고, 실행 명령은 저장소 루트 기준이다.
선·오차 막대·히스토그램·행렬·극좌표 예제는 컬럼을 등록한다(극좌표는 `Config.polar`를 켠 같은
컬럼 경로다). 원형·범주형 막대·박스플롯·레이더는 각 차트의 소량 데이터 모델을 직접 만든다. 박스플롯의 사분위수 등 통계값은 호출자가 제공한다.

정지 예제는 PNG를 만들고, 애니메이션 예제는 프레임별 PNG를 만든다.
편집 예제는 실제 창을 열어 선택·이동·영역 조정을 보여준다.
웹 예제의 이벤트 연결과 Rust 렌더러의 API는 별도로 구분한다.
