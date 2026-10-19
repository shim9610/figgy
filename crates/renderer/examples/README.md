# Chart examples

Run these commands from the workspace root with the pinned Rust toolchain.
Headless examples need a working GPU adapter, but no window or browser.
They use the default adapter selection and do not force a software driver. On a
machine without a GPU, install a software Vulkan driver such as Mesa lavapipe and
select it as described in
[the gallery notes](../GALLERY.md#generate-pngs-and-animation-frames). Each
headless example takes an output path as its first argument and writes PNG files
there.

- [Gallery images](#gallery-images): every image in the README, with the
  function that draws it.
- [All headless examples](#all-headless-examples): what each one writes,
  including outputs that are not in the README.
- [Interactive examples](#interactive-examples): native windows and live labs.
- [Review and measurement tools](#review-and-measurement-tools).

## Gallery images

Every README image is a file in
[`assets/`](https://github.com/shim9610/figgy/tree/master/crates/renderer/assets). The "Drawn by" column
names the example and the function that draws the image. Examples that build a
list of cases in `main` are cited by case name; the case name is also the output
file name. To refresh an image, run the example's command from
[the next table](#all-headless-examples), inspect the output and copy it into
`assets/` under the asset name.

### Cartesian charts

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-line.png" width="160" alt="Sine curve"> | `gallery-line.png` | [line.rs](line.rs), `main` | The complete minimal setup: two columns, one line series, one PNG |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-errorbars.png" width="160" alt="Response curves with error bars"> | `gallery-errorbars.png` | [error_bars.rs](error_bars.rs), `render` | Lines, markers and symmetric error bars from X, Y and error columns |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-histogram.png" width="160" alt="Vertical histogram"> | `gallery-histogram.png` | [histogram.rs](histogram.rs), `render` | 40 supplied bins; the renderer draws the counts it is given |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-histogram-horizontal.png" width="160" alt="Horizontal histogram"> | `gallery-histogram-horizontal.png` | [histogram.rs](histogram.rs), `render` | The same bins with horizontal bars |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-contours.png" width="160" alt="Heatmap with labelled contours"> | `gallery-contours.png` | [heatmap_contours.rs](heatmap_contours.rs), `render` | An interpolated matrix fill, labelled isolines with line gaps, a colourbar |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-wave.gif" width="160" alt="Travelling waves"> | `gallery-wave.gif` | [animated_lines.rs](animated_lines.rs), `render` | Columns replaced every frame; writes `wave/NNN.png` frames, encoded as described in [the gallery notes](../GALLERY.md#encode-the-gifs) |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-orbit.gif" width="160" alt="Lissajous orbit"> | `gallery-orbit.gif` | [animated_lines.rs](animated_lines.rs), `render` | A moving trail and a highlighted point; writes `orbit/NNN.png` frames |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-polar.png" width="160" alt="Antenna pattern in polar coordinates"> | `gallery-polar.png` | [polar_plot.rs](polar_plot.rs), case `01-antenna-pattern` | A line filled to the centre in polar coordinates, degree labels |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-polar-compass.png" width="160" alt="Compass rose"> | `gallery-polar-compass.png` | [polar_plot.rs](polar_plot.rs), case `02-compass-rose` | Clockwise from the top, arc segments, radius labels on the 45° ray |

### Cartesian features

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-wind.png" width="160" alt="Wind field over a heatmap"> | `gallery-wind.png` | [vector_field.rs](vector_field.rs), `wind` | Grid vectors from two matrices over a speed heatmap |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-currents.png" width="160" alt="Surface currents"> | `gallery-currents.png` | [vector_field.rs](vector_field.rs), `currents` | Point vectors coloured by a speed column, scaled with `fit_scale` |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-colormap.png" width="160" alt="Colour-mapped particles"> | `gallery-colormap.png` | [value_maps.rs](value_maps.rs), `colormap_scatter` | `point_color_map`: points coloured by an age column through the colourbar |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-bubbles.png" width="160" alt="Bubble chart"> | `gallery-bubbles.png` | [value_maps.rs](value_maps.rs), `bubbles` | `point_size_map`: marker area follows a population column |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-forecast.png" width="160" alt="Forecast bands"> | `gallery-forecast.png` | [area_step.rs](area_step.rs), `forecast` | `LineFill::ToSeries` bands; NaN samples break each line |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-thermostat.png" width="160" alt="Step schedule"> | `gallery-thermostat.png` | [area_step.rs](area_step.rs), `thermostat` | A `StepPost` line filled to a baseline |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-twin-axes.png" width="160" alt="Twin Y axes"> | `gallery-twin-axes.png` | [overlay_axes.rs](overlay_axes.rs), `twin_axes` | An overlay layer with its own right Y axis, exported as a document |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-subplots.png" width="160" alt="Small multiples"> | `gallery-subplots.png` | [subplots.rs](subplots.rs), `small_multiples` | A 2 × 2 grid: the parent and three subplots linked to its X axis, each with its own Y range |

### Bounded charts

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-donut.png" width="160" alt="Ceramic donut"> | `gallery-donut.png` | [radial_gallery.rs](radial_gallery.rs), case `ceramic` | Named values, rounded edges, an oblique view |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-categorical.png" width="160" alt="Grouped bars"> | `gallery-categorical.png` | [categorical_gallery.rs](categorical_gallery.rs), case `grouped-matte` | Two series across four categories |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-boxplot.png" width="160" alt="Box plot"> | `gallery-boxplot.png` | [boxplot_gallery.rs](boxplot_gallery.rs), case `01-classic-matte` | Supplied five-number summaries and outliers |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-radar.png" width="160" alt="Radar chart"> | `gallery-radar.png` | [radar_gallery.rs](radar_gallery.rs), case `01-spider-web` | Two series over six categories on a polygon grid |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/gallery-radar-circle.png" width="160" alt="Radar chart with value labels"> | `gallery-radar-circle.png` | [radar_gallery.rs](radar_gallery.rs), case `02-circle-values` | A circular grid with a value label at every vertex |

### Draw styles

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-precise.png" width="160" alt="Precise style"> | `style-growth-response-precise.png` | [precise_sketch_preview.rs](precise_sketch_preview.rs), `main` | `DrawStyle::Precise` |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-sketch.png" width="160" alt="Sketch style"> | `style-growth-response-sketch.png` | [precise_sketch_preview.rs](precise_sketch_preview.rs), `main` | `DrawStyle::Sketch` on the same data |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-milkyway.png" width="160" alt="Milkyway style"> | `style-growth-response-milkyway.png` | [milkyway_line_preview.rs](milkyway_line_preview.rs), `main` | Milkyway on `Line` series: stars and nebula only |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/style-growth-response-constellation.png" width="160" alt="Constellation style"> | `style-growth-response-constellation.png` | [constellation_preview.rs](constellation_preview.rs), `main` | Constellation on `ScatterLine` series: stars at the source samples |

### Visual concepts

These are recipes built from the public APIs, not theme presets. Each function
in [gallery_concepts.rs](gallery_concepts.rs) holds its data and styling.

| Image | Asset | Drawn by | Shows |
|---|---|---|---|
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-observability.png" width="160" alt="Dark dashboard"> | `concept-observability.png` | `observability` | A dark service-monitoring card |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-revenue.png" width="160" alt="Revenue stacks"> | `concept-revenue.png` | `revenue` | Rounded horizontal stacked bars with inside labels |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-editorial.png" width="160" alt="Editorial sketch"> | `concept-editorial.png` | `editorial` | A warm paper-coloured Sketch figure |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-pop-art.png" width="160" alt="Toon pie"> | `concept-pop-art.png` | `radial_concepts` | A toon-shaded pie with outlines |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-portfolio.png" width="160" alt="Pearl donut"> | `concept-portfolio.png` | `radial_concepts` | A pearl donut from the same slice model |
| <img src="https://raw.githubusercontent.com/shim9610/figgy/master/crates/renderer/assets/concept-terrain.png" width="160" alt="Terrain"> | `concept-terrain.png` | `terrain` | A custom colour ramp under fine isolines |

## All headless examples

| Example | Writes | Command |
|---|---|---|
| [line.rs](line.rs) | One PNG at the given path | `cargo run --locked -p figgy-renderer --example line -- target/line.png` |
| [error_bars.rs](error_bars.rs) | `gallery-errorbars.png` | `cargo run --locked -p figgy-renderer --example error_bars -- target/error-bars` |
| [histogram.rs](histogram.rs) | `gallery-histogram.png`, `gallery-histogram-horizontal.png` | `cargo run --locked -p figgy-renderer --example histogram -- target/histogram` |
| [heatmap_contours.rs](heatmap_contours.rs) | `gallery-contours.png` | `cargo run --locked -p figgy-renderer --example heatmap_contours -- target/heatmap-contours` |
| [animated_lines.rs](animated_lines.rs) | `wave/NNN.png`, `orbit/NNN.png`; the second argument is the frame count | `cargo run --locked -p figgy-renderer --example animated_lines -- target/animation 48` |
| [readme_gallery.rs](readme_gallery.rs) | The outputs of `error_bars`, `histogram`, `heatmap_contours` and `animated_lines` from one shared device | `cargo run --locked -p figgy-renderer --example readme_gallery -- target/readme-gallery` |
| [polar_plot.rs](polar_plot.rs) | `01-antenna-pattern.png`, `02-compass-rose.png`, `03-spiral-arcs.png` (a dashed spiral with arc segments), `04-sector-radians.png` (a half-circle sector in radians) | `cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot` |
| [vector_field.rs](vector_field.rs) | `gallery-wind.png`, `gallery-currents.png`, and `gradient.png` from `gradient`: grid arrows in data units from their tails, coloured by a third matrix (`VectorColorMap::Grid`) | `cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field` |
| [value_maps.rs](value_maps.rs) | `gallery-colormap.png`, `gallery-bubbles.png` | `cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps` |
| [area_step.rs](area_step.rs) | `gallery-forecast.png`, `gallery-thermostat.png`, and `steps.png` from `step_modes`: `StepPre`, `StepPost` and `StepMid` on the same samples | `cargo run --locked -p figgy-renderer --example area_step -- target/area-step` |
| [overlay_axes.rs](overlay_axes.rs) | `gallery-twin-axes.png`; also checks the overlay's hit-test ids | `cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes` |
| [subplots.rs](subplots.rs) | `gallery-subplots.png`, and `mixed-axes.png` from `mixed_axes`: one linked axis, a different X column, and a twin Y axis inside a subplot | `cargo run --locked -p figgy-renderer --example subplots -- target/subplots` |
| [gallery_concepts.rs](gallery_concepts.rs) | The six `concept-*.png` images | `cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts` |
| [radial_gallery.rs](radial_gallery.rs) | One PNG per material (`matte`, `ceramic`, `satin-metal`, `toon`, `enamel`, `brushed-metal`, `paper`, `plywood`, `hatch`, `pearl`), plus `outlined-raised-pie`, `corners-sharp`, `corners-rounded`, `individual-edit`, `pie-of-pie` and the hover frames `hover-00` to `hover-19` | `cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery` |
| [categorical_gallery.rs](categorical_gallery.rs) | `grouped-matte`, `horizontal-satin`, `stacked-enamel`, `percent-outline`, `signed-stacked` | `cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery` |
| [boxplot_gallery.rs](boxplot_gallery.rs) | `01-classic-matte`, `02-satin`, `03-horizontal`, `04-grouped-notch`, `05-mean-selection` | `cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery` |
| [radar_gallery.rs](radar_gallery.rs) | `01-spider-web`, `02-circle-values`, `03-fixed-axis`, `04-four-series`, `05-selection` | `cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery` |
| [bounded_titles.rs](bounded_titles.rs) | `pie.png`, `bar.png`, `box.png`, `radar.png`: rich titles on the bounded charts | `cargo run --locked -p figgy-renderer --example bounded_titles -- target/bounded-titles` |
| [precise_sketch_preview.rs](precise_sketch_preview.rs) | `style-growth-response-precise.png`, `style-growth-response-sketch.png` | `cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch` |
| [milkyway_line_preview.rs](milkyway_line_preview.rs) | One PNG at the given path | `cargo run --locked -p figgy-renderer --example milkyway_line_preview -- target/milkyway-preview.png` |
| [constellation_preview.rs](constellation_preview.rs) | One PNG at the given path | `cargo run --locked -p figgy-renderer --example constellation_preview -- target/constellation-preview.png` |

[support/gallery.rs](support/gallery.rs) holds device setup, a plain chart
frame and PNG checks. [support/styled.rs](support/styled.rs) holds the shared
chrome, colourbar and export of the 1200 × 800 designed scenes. Each chart's
data, series and settings are in its own example file.

## Interactive examples

These open native windows and show host event handling as well as drawing.

```bash
cargo run --locked -p figgy-renderer --example winit_simple
cargo run --locked -p figgy-renderer --features egui_demo --example egui_embed
cargo run --locked -p figgy-renderer --features egui_demo --example radial_editor
cargo run --locked -p figgy-renderer --features egui_demo --example categorical_editor
cargo run --locked -p figgy-renderer --features egui_demo --example boxplot_editor
cargo run --locked -p figgy-renderer --features egui_demo --example radar_editor
cargo run --locked --release -p figgy-renderer --features egui_demo --example ssot_lab
cargo run --locked --release -p figgy-renderer --features egui_demo --example streaming_capacity_demo
cargo run --locked -p figgy-renderer --features egui_demo --example constellation_lab
```

- **winit_simple** and **egui_embed** draw three panels in a host window and
  handle pointer input. `winit_simple` also selects, drags and resizes chart
  elements.
- **The four editors** select, edit and drag the parts of a bounded chart.
  `Edit plot area` shows eight resize handles.
- **ssot_lab** edits four charts live, one per draw style. All four series
  read one X column from the pool, and vertically adjacent panels share their
  X range.
- **streaming_capacity_demo** generates its data lazily through `ColumnSource`
  and draws every point. Add `-- --benchmark` to print timings for the default
  source.
- **constellation_lab** binds every `ConstellationOptions` field to a slider.

[Interaction contracts](../INTERACTION.md) explain logical coordinates, hit
priority, incremental deltas, rejected edits and export state.
The [browser integration example](https://github.com/shim9610/figgy/blob/master/crates/web/README.md)
is a separate WASM consumer. It demonstrates a host bridge and does not expose
every native chart API.

## Review and measurement tools

These are for checking renderer output and cost while changing it. Their
images are not in the README.

| Example | Purpose | Command |
|---|---|---|
| [constellation_demo.rs](constellation_demo.rs) | Despite its name, renders the Milkyway style: parameter sweeps of the line stars and ribbon, spectrum and sampling checks, scatter planets and errorbar jets, into `target/constellation_demo/` | `cargo run --locked -p figgy-renderer --example constellation_demo` |
| [sketch_demo.rs](sketch_demo.rs) | Precise and Sketch exports of one chart, and a Sketch tour with line, scatter-line and errorbar series, into `target/sketch_demo/` | `cargo run --locked -p figgy-renderer --example sketch_demo` |
| [raster_bench.rs](raster_bench.rs) | Prints the CPU raster time of the grid and decoration layers for Precise and Milkyway at two sizes; needs no GPU | `cargo run --locked --release -p figgy-renderer --example raster_bench` |

## 한국어 안내

- **갤러리 그림**: README의 모든 그림은 `assets/`에 있다. 위 표의 "Drawn by" 열이 그 그림을
  그리는 예제와 함수를 가리킨다. `main`에서 사례 목록을 만드는 예제는 사례 이름으로 적었고,
  사례 이름이 곧 출력 파일 이름이다.
- **실행**: 명령은 저장소 루트 기준이다. 첫 인자가 출력 경로이며, 창 없이 GPU 어댑터만 있으면 된다.
  GPU가 없는 장비에서는 Mesa lavapipe 같은 소프트웨어 Vulkan 드라이버를 설치해 쓴다.
- **README에 없는 출력**: `steps.png`(계단 보간 세 가지 비교), `gradient.png`(데이터 단위
  격자 화살표와 격자 색 지도), `mixed-axes.png`(서브플롯 안의 보조 Y축)처럼 기능 확인용
  출력도 같은 예제가 함께 만든다.
- **데이터 경로**: 선·오차 막대·히스토그램·행렬·극좌표·벡터장 예제는 컬럼을 등록한다.
  원형·범주형 막대·박스플롯·레이더는 각 차트의 소량 데이터 모델을 직접 만든다. 박스플롯의
  사분위수 등 통계값은 호출자가 제공한다.
- **창 예제와 도구**: 편집 예제는 실제 창을 열어 선택·이동·영역 조정을 보여준다.
  `constellation_demo`, `sketch_demo`, `raster_bench`는 렌더러를 바꿀 때 출력과 비용을
  확인하는 도구다.
