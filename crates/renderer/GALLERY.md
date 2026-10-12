# Native chart gallery

Each README image links to its [runnable Rust example](examples/README.md).
[`readme_gallery.rs`](examples/readme_gallery.rs) runs the Cartesian examples
with one shared device. They use the native `Renderer` with an offscreen RGBA target, upload columns through
the resident pool, and exports through `export_panel_rgba_with_clear_async` and
`encode_png`. No browser or window is needed. Axes, text, grid and legends follow
the same CPU-raster/GPU-compositing path as other figgy charts.

All data is synthetic. These images demonstrate plotting features, not measured
scientific results. The error bars are deliberately chosen widths, not confidence
intervals inferred from observations.

| Output | Data and rendering | Size |
|---|---|---|
| `gallery-line.png` | Minimal sine curve from `line.rs` | 960 × 600 |
| `gallery-errorbars.png` | Two exponential responses, 18 samples each; lines, markers and symmetric error bars | 960 × 600 |
| `gallery-histogram.png` | 40 explicit bins over [−4, 4]; rounded counts from a two-peak analytic mixture | 960 × 600 |
| `gallery-histogram-horizontal.png` | The same uploaded bins and counts, with horizontal bars | 960 × 600 |
| `gallery-contours.png` | 81 × 81 samples of two Gaussian peaks; interpolated Viridis fill, 10 isolines with labels and line gaps, shared colourbar | 960 × 640 |
| `gallery-wave.gif` | Two travelling sine waves, 401 samples each; columns replaced for every phase | 720 × 460 |
| `gallery-orbit.gif` | A 2:3 Lissajous curve, a moving trail and a highlighted current point | 720 × 460 |
| `gallery-donut.png` | `ceramic.png` from `radial_gallery.rs`: named values, rounded edges and an oblique view | 900 × 620 |
| `gallery-categorical.png` | `grouped-matte.png` from `categorical_gallery.rs`: two series across four categories | 900 × 620 |
| `gallery-boxplot.png` | `01-classic-matte.png` from `boxplot_gallery.rs`: supplied summaries and outliers | 960 × 640 |

The first four Cartesian chart recipes also run independently as `line`,
`error_bars`, `histogram` and `heatmap_contours`. `animated_lines` writes wave and
orbit frames. The shared support file contains setup and export checks, not the
chart definitions. README galleries use four columns, preserve each image's aspect ratio, align
images at the top of each row and link directly to the generating code.

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
uses the existing Cartesian, categorical and radial APIs without changing their
defaults or data contracts. See [configuration](CONFIG.md), [categorical bars](CATEGORICAL.md)
and [radial materials](RADIAL.md) to adapt the recipes.

The PNGs come directly from native export at their displayed source resolution:
there is no image-generation step, compositing or upscaling. The export clear
color supplies the Cartesian background; bounded charts use their model's
`background` field. All six share the same aspect ratio for aligned README rows.
Existing Milkyway, Constellation and scientific examples remain in the gallery.

## Generate PNGs and animation frames

Run from the workspace root, using the pinned Rust 1.99.0 toolchain and committed
lockfile. Choose an empty output directory so frames from another run cannot be
mixed into an animation.

```bash
cargo run --locked -p figgy-renderer --example readme_gallery -- target/readme-gallery
```

This writes the four static PNGs and `wave/000.png` … `wave/047.png` plus
`orbit/000.png` … `orbit/047.png`. The optional second argument selects 2–240
frames; the default is 48. Phases cover one period without repeating the endpoint.
The example uses fixed axis ranges so the animation does not rescale each frame.

The example reports the selected adapter, dimensions and coloured-pixel count.
Each export must succeed, contain opaque pixels on its white background, and
contain at least 500 coloured pixels. Adapter, allocation and export failures
return an error; missing GPU support is not treated as a successful run.
These checks catch blank exports; visual review is still needed for layout.

Normal native adapter selection is used. A GPU and working driver are required,
or an explicitly configured software Vulkan adapter on machines without a GPU.
The example does not force a software driver. The checked-in gallery was rendered
on Linux with Vulkan llvmpipe (LLVM 19.1.7, Mesa 25.0.7), a software adapter;
it is not evidence of hardware GPU performance or cross-platform pixel equality.

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
copy the four Cartesian `gallery-*.png` and two `gallery-*.gif` files into
[`assets/`](assets/). Keep the numbered intermediate frames outside source control.
The source example is the authority for formulas, styles and export settings.

Generate the additional still images with:

```bash
cargo run --locked -p figgy-renderer --example line -- target/readme-gallery/gallery-line.png
cargo run --locked -p figgy-renderer --example radial_gallery -- target/radial-gallery
cargo run --locked -p figgy-renderer --example categorical_gallery -- target/categorical-gallery
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
```

Copy the selected outputs under the filenames in the table. The bounded examples
use `RadialRenderer`, `CategoricalRenderer` and `BoxPlotRenderer`, respectively;
their small-data models do not use the Cartesian column pool.

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

은하수 미리보기는 위의 별도 예제로 만든다. 두 곡선을 `Line`으로 등록하면 곡선을 따라 별과 성운 띠만 그린다. 산점도 기호를 사용하지 않으므로 행성이 나타나지 않고, 오차 막대도 없어 제트가 나타나지 않는다. `planet_rim = 0`은 행성을 없애는 설정이 아니다. 위 옵션은 미리보기 예제에만 적용되며 렌더러의 기본값은 바꾸지 않는다.

## Constellation preview

```bash
cargo run --locked -p figgy-renderer --example constellation_preview -- target/constellation-preview.png
```

[`constellation_preview.rs`](examples/constellation_preview.rs) renders two
synthetic growth curves with ten samples each as `DataRenderType::ScatterLine`.
Stars occupy those original sample positions, with translucent connecting lines.
The example sets `star_opacity = 1` and `line_opacity = 0.45`, exports a
1280 × 1024 PNG, and checks for visible data against an empty chart.

The README links this example and the line-only Milkyway example directly from
their images. Both images have the same dimensions and display at the same width.
After inspection, copy the output to `assets/style-growth-response-constellation.png`.

## 한국어 사용법

이 갤러리는 네이티브 렌더러가 출력한 PNG로 만들었다. `readme_gallery.rs`는 GPU 풀에 데이터를 올리고, 데이터와 축·눈금·텍스트를 합성해 PNG로 저장한다. 창이나 브라우저를 열지 않고 실행할 수 있다.

- **정지 이미지 4종**: 응답 곡선과 오차 막대, 구간 40개의 세로·가로 히스토그램, 81×81 행렬의 보간 히트맵과 라벨이 있는 등고선이다. 모두 예제용으로 생성한 데이터이며 오차 막대의 폭도 설명을 위한 값이다.
- **애니메이션 2종**: 이동하는 파형과 2:3 위상 궤적이다. 프레임마다 컬럼 값을 갱신하고 PNG를 다시 출력한다. 축 범위는 고정하며 처음과 같은 위상이 되는 마지막 프레임은 제외한다.
- **생성 순서**: 저장소 루트에서 위 `cargo run --locked` 명령을 실행하면 정지 PNG 4개와 애니메이션 프레임 96개를 만든다. 이어서 FFmpeg 명령으로 GIF 2개를 생성한다. 이전 프레임이 섞이지 않도록 빈 출력 디렉터리를 사용한다.
- **재생 설정**: GIF 하나당 48프레임을 초당 12프레임으로 약 4초마다 반복한다. GIF는 색을 팔레트로 줄이므로 픽셀 검사는 원본 PNG로 한다. GIF의 재생 속도는 렌더링 성능을 나타내지 않는다.
- **검증 환경**: 일반 장비에서는 기본 GPU 선택을 따른다. 이 이미지는 실제 GPU가 없는 Linux에서 Vulkan llvmpipe로 검증했다. 예제 자체가 소프트웨어 GPU를 강제하지는 않는다. 실제 GPU의 성능이나 다른 OS에서 같은 픽셀값이 나오는지는 확인하지 않았다.

갤러리를 갱신할 때는 출력 오류 여부, 불투명 배경과 유효한 색상 픽셀, 축 제목·눈금·범례·색상 막대 배치를 확인한다. GIF도 디코딩해 프레임 수, 프레임 간 변화, 반복 설정을 검사한다. 위 표의 출력 파일만 `assets/`에 복사하고 중간 프레임은 소스 관리에 넣지 않는다.

일반 Rust 사용법은 [빠른 시작](../../README.ko.md#빠른-시작)과 [상세 사용법](USAGE.md), 브라우저 통합 예제의 초기화·수명 관리·입출력은 [WASM.md](WASM.md), JSON 형식은 [SCHEMA.md](../web/SCHEMA.md)를 참고한다.

The horizontal PNG is an additional orientation example. For the column bindings, axis settings and live updates, see [Histogram orientation](HISTOGRAM.md).

가로 히스토그램 PNG는 방향 설정을 보여주는 추가 예제다. 컬럼 연결과 축 설정, 기존 차트의 방향 변경은 [히스토그램 사용법](HISTOGRAM.md)을 참고한다.
