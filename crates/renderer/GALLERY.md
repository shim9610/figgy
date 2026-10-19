# Native chart gallery

Each README image links to the example file that draws it. The
[example index](examples/README.md#gallery-images) names the function for every
image.
[`readme_gallery.rs`](examples/readme_gallery.rs) runs the `error_bars`,
`histogram`, `heatmap_contours` and `animated_lines` examples with one shared
device. They use the native `Renderer` with an offscreen RGBA target, upload columns through
the resident pool, and export through `export_panel_rgba_with_clear_async` and
`encode_png`. No browser or window is needed. Axes, text, grid and legends follow
the same CPU-raster/GPU-compositing path as other figgy charts.

Radar charts, polar coordinates, vector fields, point colour and size maps, area
fills, step lines, overlay layers and subplots are on the repository's master
branch and are not in the published figgy-renderer 0.17.0. The shared `line` and
`scatter` helpers in [support/gallery.rs](examples/support/gallery.rs) set the
new line-fill, interpolation and point-map fields, so recipes that use them do
not compile against 0.17.0. Until the next release, depend on the repository:

```toml
renderer = { package = "figgy-renderer", git = "https://github.com/shim9610/figgy" }
```

All data is synthetic. These images demonstrate plotting features, not measured
scientific results. The error bars are deliberately chosen widths, not confidence
intervals inferred from observations.

| Output | Data and rendering | Size |
|---|---|---|
| `gallery-line.png` | Minimal sine curve from `line.rs` | 960 × 600 |
| `gallery-errorbars.png` | Two exponential responses, 18 samples each; lines, markers and symmetric error bars | 960 × 600 |
| `gallery-histogram.png` | 40 explicit bins over [−4, 4]; rounded counts from a two-peak analytic mixture | 960 × 600 |
| `gallery-histogram-horizontal.png` | The same uploaded bins and counts, with horizontal bars; column bindings, axis settings and switching an existing chart are in [Histogram orientation](HISTOGRAM.md) | 960 × 600 |
| `gallery-contours.png` | 81 × 81 samples of two Gaussian peaks; interpolated Viridis fill, 10 isolines with labels and line gaps, shared colourbar | 960 × 640 |
| `gallery-wave.gif` | Two travelling sine waves, 401 samples each; columns replaced for every phase | 720 × 460 |
| `gallery-orbit.gif` | A 2:3 Lissajous curve, a moving trail and a highlighted current point | 720 × 460 |
| `gallery-donut.png` | `ceramic.png` from `radial_gallery.rs`: named values, rounded edges and an oblique view | 900 × 620 |
| `gallery-categorical.png` | `grouped-matte.png` from `categorical_gallery.rs`: two series across four categories | 900 × 620 |
| `gallery-boxplot.png` | `01-classic-matte.png` from `boxplot_gallery.rs`: supplied summaries and outliers | 960 × 640 |
| `gallery-radar.png` | `01-spider-web.png` from `radar_gallery.rs`: two series over six categories on a polygon grid | 960 × 640 |
| `gallery-radar-circle.png` | `02-circle-values.png` from `radar_gallery.rs`: the same series on a circular grid with a value label at every vertex | 960 × 640 |
| `gallery-polar.png` | `01-antenna-pattern.png` from `polar_plot.rs`: a filled line in polar coordinates with degree labels | 960 × 640 |
| `gallery-polar-compass.png` | `02-compass-rose.png` from `polar_plot.rs`: a scatter-line clockwise from the top with arc segments, the radius labels on the 45° ray | 960 × 640 |

`error_bars`, `histogram`, `heatmap_contours` and `animated_lines` also run on
their own; `animated_lines` writes the wave and orbit frames. `line` is not part
of `readme_gallery` and runs only on its own.
[support/gallery.rs](examples/support/gallery.rs) holds device setup,
presentation defaults and PNG checks, not the chart definitions. To add an image
to a README gallery row, keep four images per row (`width="23%" align="top"`, no
fixed height) and link the image to the example that writes it.

## Feature scenes

The Cartesian feature examples draw the designed 1200 × 800 scenes in the
README. [support/styled.rs](examples/support/styled.rs) holds their shared
chrome: no axis lines or ticks, a horizontal grid and a quiet legend. Some scenes
hide the legend or the grid, or add vertical grid lines; the data and series stay
in each example.

| Output | Example and function | Scene |
|---|---|---|
| `gallery-wind.png` | [vector_field.rs](examples/vector_field.rs), `wind` | A sea breeze: speed as a smooth field on a custom ramp, direction as white arrows on a coarser grid of the same coordinates |
| `gallery-currents.png` | [vector_field.rs](examples/vector_field.rs), `currents` | Surface currents at jittered drifter positions; arrow length follows the (u, v) components, arrow colour follows a host-supplied speed column through the colourbar |
| `gallery-colormap.png` | [value_maps.rs](examples/value_maps.rs), `colormap_scatter` | 1400 particles along two spiral arms, coloured by an age column |
| `gallery-bubbles.png` | [value_maps.rs](examples/value_maps.rs), `bubbles` | Three regions of cities; marker area follows population over one shared domain, translucent fills |
| `gallery-forecast.png` | [area_step.rs](examples/area_step.rs), `forecast` | History solid, forecast dashed, 80% and 95% bands filled between their edge series; samples outside each part are NaN |
| `gallery-thermostat.png` | [area_step.rs](examples/area_step.rs), `thermostat` | A setpoint schedule as a `StepPost` line filled to 14 °C, the room temperature over it |
| `gallery-twin-axes.png` | [overlay_axes.rs](examples/overlay_axes.rs), `twin_axes` | Voltage on the left axis with a light fill, current on an overlay layer's right axis |
| `gallery-subplots.png` | [subplots.rs](examples/subplots.rs), `small_multiples` | Four sensors in a 2 × 2 grid sharing the hour axis, each with its own range, colour and fill |

`twin_axes` and `small_multiples` export with
`export_document_rgba_with_clear_async`: the panel export draws the base panel
only, without overlay layers or subplots.

The same examples also write plain feature checks that are not in the README:
`steps.png` compares the three step modes, `gradient.png` draws grid arrows in
data units coloured by a third matrix, and `mixed-axes.png` puts a twin Y axis
inside a subplot.

## Visual concepts

[`gallery_concepts.rs`](examples/gallery_concepts.rs) renders six complete chart
images at **1200 × 800**. Each named function contains its data and styling choices;
`cartesian` shares the axis/legend setup and `save` checks and encodes the output.
These are examples to adapt, not additional built-in theme names.

```bash
cargo run --locked -p figgy-renderer --example gallery_concepts -- target/concepts
```

| Image / function | Concept | Settings to try |
|---|---|---|
| `concept-observability.png` / `observability` | Dark service-monitoring card; two synthetic request-rate curves | Navy clear color, mint/lavender lines, horizontal grid only, hidden axis lines and ticks |
| `concept-revenue.png` / `revenue` | Light product-analytics card; four regional revenue stacks | `Horizontal` + `Stacked`, inside value labels, rounded exposed ends, matte bars, no grid |
| `concept-editorial.png` / `editorial` | Warm editorial chart; two community-growth series | `DrawStyle::Sketch`, fixed wobble seed, paper-colored background, sparse round markers |
| `concept-pop-art.png` / `radial_concepts` | Bright, outlined pie; time allocation | `Toon`, 45° tilt, shallow extrusion, rim/separator outlines, small slice gaps |
| `concept-portfolio.png` / `radial_concepts` | Dark presentation chart; asset allocation | `Pearl`, 55° tilt, 0.57 hole ratio, rounded corners, bevel, soft highlights |
| `concept-terrain.png` / `terrain` | Topographic color study; an imaginary landscape | A custom forest-to-sand `ColorMap`, interpolated matrix fill and 17 fine isolines |

The dashboard examples are chart images suitable for embedding in a card. Page
layout, KPI widgets and application controls belong to the host. The example
uses only the public Cartesian, categorical and radial APIs. See
[configuration](CONFIG.md), [categorical bars](CATEGORICAL.md) and
[radial materials](RADIAL.md) to adapt the recipes.

The PNGs are the native export, written without resizing or post-processing. The export clear color supplies the Cartesian background; the
categorical and radial charts use their model's `background` field.

## Generate PNGs and animation frames

Run from the workspace root, using the pinned Rust 1.99.0 toolchain and committed
lockfile. Choose an empty output directory so frames from another run cannot be
mixed into an animation.

```bash
cargo run --locked -p figgy-renderer --example readme_gallery -- target/readme-gallery
```

This writes four static PNGs (`gallery-errorbars.png`, `gallery-histogram.png`,
`gallery-histogram-horizontal.png` and `gallery-contours.png`) and the frames
`wave/000.png` … `wave/047.png` and `orbit/000.png` … `orbit/047.png`. The
optional second argument selects 2–240 frames; the default is 48. Phases cover
one period without repeating the endpoint. The example uses fixed axis ranges so
the animation does not rescale each frame.

The example reports the selected adapter, dimensions and coloured-pixel count.
Each export must succeed, be fully opaque on its white background, and contain
at least 500 coloured pixels. Adapter, allocation and export failures return an
error; missing GPU support is not treated as a successful run. These checks catch
blank exports; visual review is still needed for layout.

Normal native adapter selection is used, and the example does not force a
software driver. A working adapter is required: a GPU with its driver, or, on a
machine without a GPU, a software Vulkan driver selected through the Vulkan
loader. For example, install Mesa lavapipe (`mesa-vulkan-drivers` on Debian and
Ubuntu) and set `VK_DRIVER_FILES` and `VK_ICD_FILENAMES` to its `lvp_icd*.json`
file, as the "Configure headless Linux rendering" step of the
[CI workflow](https://github.com/shim9610/figgy/blob/master/.github/workflows/ci.yml)
does. The checked-in images were rendered on Linux with Mesa lavapipe, a
software Vulkan adapter that reports itself as llvmpipe, not a hardware GPU;
they are not evidence of hardware GPU performance or cross-platform pixel
equality.

## Encode the GIFs

With FFmpeg installed, run from the workspace root after the default 48-frame export:

```bash
for name in wave orbit; do
  ffmpeg -y -framerate 12 -i "target/readme-gallery/$name/%03d.png" \
    -filter_complex_threads 1 \
    -filter_complex '[0:v]split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a' \
    -frames:v 48 -loop 0 "target/readme-gallery/gallery-$name.gif"
done
```

Each GIF loops forever and represents four seconds. GIF delays are quantized to
hundredths of a second, so individual frame delays alternate around 1/12 second.
The 128-colour palette reduces file size; the original exported PNGs retain the
renderer’s RGBA output and should be used for pixel inspection. Playback rate is
chosen for the README and says nothing about rendering throughput. If a different
frame count was requested, change `-frames:v` to match; duration is count / 12.

To refresh the README, inspect the PNGs and the decoded animation frames, then
copy those four static PNGs and the two `gallery-*.gif` files into
[`assets/`](https://github.com/shim9610/figgy/tree/master/crates/renderer/assets). Keep the numbered intermediate frames outside source control.
The source example is the authority for formulas, styles and export settings.

## Generate the other still images

Run from the workspace root after the `readme_gallery` run: `line` writes to the
given path and does not create `target/readme-gallery`. The
[example index](examples/README.md#all-headless-examples) lists every file each
example writes.

```bash
cargo run --locked -p figgy-renderer --example line -- target/readme-gallery/gallery-line.png
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
cargo run --locked -p figgy-renderer --example radar_gallery -- target/radar-gallery
cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
cargo run --locked -p figgy-renderer --example vector_field -- target/vector-field
cargo run --locked -p figgy-renderer --example value_maps -- target/value-maps
cargo run --locked -p figgy-renderer --example area_step -- target/area-step
cargo run --locked -p figgy-renderer --example overlay_axes -- target/overlay-axes
cargo run --locked -p figgy-renderer --example subplots -- target/subplots
```

Copy each output into `assets/` under its name in the table at the top of this
page. The case outputs of `radial_gallery`, `categorical_gallery`,
`boxplot_gallery`, `radar_gallery` and `polar_plot` are renamed, for example
`target/radial-gallery/ceramic.png` → `assets/gallery-donut.png`; the feature
examples and `gallery_concepts` already write their final names. The
bounded examples use `RadialRenderer`, `CategoricalRenderer`, `BoxPlotRenderer`
and `RadarRenderer`, respectively; their small-data models do not use the
Cartesian column pool.

## Precise and hand-drawn Sketch previews

```bash
cargo run --locked -p figgy-renderer --example precise_sketch_preview -- target/precise-sketch
```

[`precise_sketch_preview.rs`](examples/precise_sketch_preview.rs) exports the same
two growth curves with ten samples each in Precise and Sketch mode. Sketch uses
`amplitude_px = 2.2`, `wavelength_px = 55` and `seed = 1` for reproducible hand-drawn
lines, axes and lettering. Both PNGs are 1280 × 1024, matching the Milkyway and
Constellation thumbnails in the four-column draw-style row. Copy the two
`style-growth-response-*.png` outputs into `assets/` after inspection.

## Milkyway line-only preview

The Milkyway style preview uses a separate reproducible example:

```bash
cargo run --locked -p figgy-renderer --example milkyway_line_preview -- target/milkyway-preview.png
```

[`milkyway_line_preview.rs`](examples/milkyway_line_preview.rs) exports a
1280 × 1024 PNG containing two synthetic growth curves, each declared as
`DataRenderType::Line`. The line pipeline generates the stars and nebula along
arc length. There are no scatter planets or errorbar jets in this preview.
The example also compares against an empty chart to verify visible data pixels.

The selected parameters are explicit in the example: `star_density = 70`,
`star_scale = 1`, `star_brightness = 0.32`, `spread_px = 8`,
`ribbon_width_px = 24`, `ribbon_intensity = 0.08`, `structure_scale = 1`,
`faint_bias = 4.5`, `glow = 0.35`, `nebula = 0.6`, `dust = 0.7`,
`planet_rim = 0`, and `seed = 0`. These are presentation settings for this
preview; they do not change the renderer's default options. Planet removal
comes from choosing `Line`, not from setting `planet_rim` to zero.

After inspection, copy the output to `assets/style-growth-response-milkyway.png`.

## Constellation preview

```bash
cargo run --locked -p figgy-renderer --example constellation_preview -- target/constellation-preview.png
```

[`constellation_preview.rs`](examples/constellation_preview.rs) renders two
synthetic growth curves with ten samples each as `DataRenderType::ScatterLine`.
Stars occupy those original sample positions, with translucent connecting lines.
The example sets `star_opacity = 1` and `line_opacity = 0.45`, exports a
1280 × 1024 PNG, and checks for visible data against an empty chart.

After inspection, copy the output to `assets/style-growth-response-constellation.png`.

## 한국어 사용법

이 갤러리는 네이티브 렌더러가 출력한 PNG로 만들었다. `readme_gallery.rs`는 GPU 풀에 데이터를 올리고, 데이터와 축·눈금·텍스트를 합성해 PNG로 저장한다. 창이나 브라우저를 열지 않고 실행할 수 있다.

- **배포 버전**: 레이더 차트, 극좌표, 벡터장, 점 색·크기 매핑, 영역 채우기, 계단선, 오버레이 레이어, 서브플롯은 저장소 master 브랜치에 있으며 crates.io에 배포된 figgy-renderer 0.17.0에는 아직 없다. 다음 릴리스 전에는 저장소를 의존성으로 지정한다.
- **정지 이미지 4종**: 응답 곡선과 오차 막대, 구간 40개의 세로·가로 히스토그램, 81×81 행렬의 보간 히트맵과 라벨이 있는 등고선이다. 모두 예제용으로 생성한 데이터이며 오차 막대의 폭도 설명을 위한 값이다.
- **애니메이션 2종**: 이동하는 파형과 2:3 위상 궤적이다. 프레임마다 컬럼 값을 갱신하고 PNG를 다시 출력한다. 축 범위는 고정하며 처음과 같은 위상이 되는 마지막 프레임은 제외한다.
- **생성 순서**: 저장소 루트에서 `cargo run --locked -p figgy-renderer --example readme_gallery -- target/readme-gallery`를 실행하면 정지 PNG 4개와 애니메이션 프레임 96개를 만든다. 이어서 FFmpeg 명령으로 GIF 2개를 생성한다. 이전 프레임이 섞이지 않도록 빈 출력 디렉터리를 사용한다.
- **재생 설정**: GIF 하나당 48프레임을 초당 12프레임으로 약 4초마다 반복한다. GIF는 색을 팔레트로 줄이므로 픽셀 검사는 원본 PNG로 한다. GIF의 재생 속도는 렌더링 성능을 나타내지 않는다.
- **검증 환경**: 일반 장비에서는 기본 GPU 선택을 따른다. 이 이미지는 실제 GPU가 없는 Linux에서 Vulkan llvmpipe로 검증했다. 예제 자체가 소프트웨어 GPU를 강제하지는 않는다. 실제 GPU의 성능이나 다른 OS에서 같은 픽셀값이 나오는지는 확인하지 않았다.

갤러리를 갱신할 때는 출력 오류 여부, 불투명 배경과 유효한 색상 픽셀, 축 제목·눈금·범례·색상 막대 배치를 확인한다. GIF도 디코딩해 프레임 수, 프레임 간 변화, 반복 설정을 검사한다. 나머지 정지 이미지는 `line`, `radial_gallery`, `categorical_gallery`, `boxplot_gallery`, `radar_gallery`, `polar_plot`, `vector_field`, `value_maps`, `area_step`, `overlay_axes`, `subplots`, `gallery_concepts`, `precise_sketch_preview`, `milkyway_line_preview`, `constellation_preview` 예제로 만든다. 출력 파일은 각 표와 절에 적힌 이름으로 `assets/`에 복사하고, 중간 프레임은 소스 관리에 넣지 않는다.

은하수 미리보기는 두 곡선을 `Line`으로 등록하므로 곡선을 따라 별과 성운 띠만 그린다. 산점도 기호가 없어 행성이 나타나지 않고, 오차 막대가 없어 제트도 나타나지 않는다. `planet_rim = 0`은 행성을 없애는 설정이 아니다. 예제의 옵션 값은 이 미리보기에만 적용되며 렌더러 기본값은 바꾸지 않는다.

가로 히스토그램 PNG는 방향 설정을 보여주는 추가 예제다. 컬럼 연결과 축 설정, 기존 차트의 방향 변경은 [히스토그램 사용법](HISTOGRAM.md)을 참고한다.

일반 Rust 사용법은 [빠른 시작](https://github.com/shim9610/figgy/blob/master/README.ko.md#빠른-시작)과 [상세 사용법](USAGE.md), 브라우저 통합 예제의 초기화·수명 관리·입출력은 [WASM.md](WASM.md), JSON 형식은 [SCHEMA.md](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)를 참고한다.
