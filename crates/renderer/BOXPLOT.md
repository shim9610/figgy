# Box plots / 박스플롯

`BoxPlotRenderer` draws small, precomputed box-and-whisker charts. It shares the
host's wgpu device/queue and owns a separate resource ledger. This API
is independent of `ColumnSource` and automatic streaming. Multiple series are
placed side by side in each category. Stacking is not a box-plot operation.

The native and WASM renderer APIs are available; web Studio has no UI binding yet.

## Input and first render

```rust,no_run
use renderer::{Category, Color, RendererDevice, boxplot::*};
# fn example(gpu: RendererDevice) -> Result<(), Box<dyn std::error::Error>> {
let mut control = BoxSummary::new(12., 30., 42., 55., 78.);
control.outliers = vec![90.];
control.median_ci = Some([37., 47.]);
control.mean = Some(44.);
control.sample_count = Some(80);
let mut chart = BoxPlotChart {
    categories: vec![Category::new("control", "Control")],
    series: vec![BoxPlotSeries::new(
        "response", "Response", vec![Some(control)],
        Color::from_rgb8(65, 144, 208),
    )],
    value_title: "Response".into(),
    ..Default::default()
};
chart.style.material = BoxPlotMaterial::SatinMetal;
let mut renderer = BoxPlotRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm)?;
let frame = renderer.prepare(&chart, (960, 640), 1.0)?;
// frame.draw(&mut render_pass); // same-format, same-size, single-sample target
# Ok(())
# }
```

`BoxSummary::new(low, q1, median, q3, high)` receives the **actual whisker
endpoints**. The host decides whether these mean min/max, observed endpoints
inside 1.5 IQR fences, or another documented policy. The renderer does not sort
samples, calculate quartiles, or infer that policy. `mean`, `sample_count`,
`median_ci` and `outliers` are optional statistics.

- Required order: `whisker_low <= q1 <= median <= q3 <= whisker_high`, all finite.
- Supplied outliers must be finite and strictly outside the whiskers.
- CI must contain the median; it may extend beyond Q1/Q3 or the whiskers.
- `notched = true` requires an explicit CI for each affected nonmissing box.
  Extended notches retain their original endpoints, including the standard
  flipped shape outside the box. Equal CI endpoints are permitted.
- `None` is a missing category/series observation; it leaves its group slot empty.
  Equal Q1/Q3 has no filled body but still draws enabled median/whisker/cap/mean marks.
- Counts, when supplied, must be positive. Missing counts display `n=—`.

Limits: 1–64 categories, 1–16 series, at most 512 category×series entries including
missing entries, at most 128 outliers per box and 4096 total. Invalid input is
rejected before replacing the previous cached frame.

## SSOT editing and picking

```rust,no_run
# use renderer::{Color, boxplot::*};
# fn edit(chart: &mut BoxPlotChart) {
let target = BoxPlotTarget::new("control", "response");
let mut individual = BoxPlotOverride::new(target.clone());
individual.color = Some(Color::from_rgb8(42, 168, 162));
individual.style = Some(BoxPlotStyle {
    material: BoxPlotMaterial::Matte,
    outline: false,
    ..chart.style.clone()
});
individual.labels = Some(BoxPlotLabels::MedianAndCount);
// Replace an existing override with the same target instead of adding duplicates.
chart.overrides.retain(|o| o.target != target);
chart.overrides.push(individual);
chart.selected = Some(target);
chart.direction = BoxPlotDirection::Horizontal;
# }
```

Style precedence: chart → series → individual. A style override replaces the
whole style; clone the parent first when changing only one property. Clear the
`Option` to inherit again. Color and label overrides reset independently.
Category and series IDs remain stable across reordering; use `reorder_categories`
to move category labels and all aligned summary columns atomically.

`frame.hit_test([x, y])` accepts chart-local **logical** coordinates and returns a
`BoxPlotPick { target, part }`. Parts are `Box`, `Median`, `WhiskerLow/High`,
`CapLow/High`, `Mean`, and `Outlier(index)`. Outlier indices follow the supplied
list; category/series identity is stable. Hollow-marker interiors, notch holes
and geometry outside the data viewport do not pick. Later visible components
win at overlaps. The caller builds a tooltip with `chart.summary(&pick.target)`.
Set `hovered`/`selected` and prepare again for emphasis; no statistical endpoint
moves during hover. `part_rect`, `box_rect`, `plot_rect` return physical pixels.
A notched box rectangle includes extended CI; a degenerate body returns `None`.

## Styles and layout

| Option | Default / accepted values |
|---|---|
| `direction` | Vertical / Horizontal |
| `material` | Matte; also Flat, SatinMetal |
| `corner_radius` | 0 logical px, 0–3; capped by body size; notched silhouettes use the CI polygon |
| `outline`, `outline_width`, `outline_color` | true, 1.2 px (0–8), dark ink |
| `median_width`, `median_color` | 1.2 px (0–8), dark ink; zero hides median |
| `whisker_width`, `whisker_color` | 1.2 px (0–8), dark ink; zero hides whiskers/caps |
| `caps`, `cap_ratio` | true, 0.5 (0–1 of box width) |
| `notched`, `notch_depth` | false, 0.22 (0–0.45 of box width per side) |
| `show_mean`, `mean_size`, `mean_color` | false, 7 px (1–24), red diamond |
| `show_outliers`, `outlier_size`, `outlier_color` | true, 7 px (1–24), dark ink |
| `outlier_shape`, `outlier_filled` | Circle (or Square), false; hollow stroke is at most 1.2 px |
| `texture_strength`, `texture_scale`, `gloss` | 0.2 (0–1), 1 (0.1–8), 0.45 (0–1) |
| `emphasis_brightness` | 0.08 (0–0.5), additive surface brightness |
| `labels` | None / Median / MedianAndCount; labels sit beyond the high endpoint |
| `label_decimals`, `value_suffix` | 1 (0–6), empty |
| `value_range` | `None` fits all supplied statistics; `Some([min, max])` fixes a linear viewport |
| `group_width`, `box_gap` | 0.65 (0.1–0.95), 8 logical px (0–24) |
| `font_size`, `font_family` | 14 (8–32), sans-serif |
| `grid`, `legend` | true |

All colors accept RGBA channels in 0–1. Disabling the box outline does not disable
median, whiskers or outliers. Fill alpha does not alter the independent mark colors.
Materials affect only the box surface. Small corner radii stay within its bounds.
There is no perspective displacement of quartiles or whiskers.

Auto range includes **all supplied statistics**, even hidden means/outliers/CI,
so toggling their visibility does not silently rescale the chart. It adds padding
and linear nice ticks. Fixed ranges clip all data components and picking to the
plot. No logarithmic scale is exposed in this initial API. Unrepresentable ranges
or indistinguishable tick strings return a layout error rather than misleading
labels. All-missing charts get a finite default range.

Minimum logical canvas: 240×200; scale: 0.5–4. Text is rasterized at the actual
output resolution. Four geometric subpixel samples smooth the data silhouettes.
If category labels, series slots or explicitly enabled data labels do not fit,
prepare returns a layout error: enlarge the canvas, extend the range for data
labels, shorten labels, or hide them. Labels never silently overlap.

## Ownership, caching, export

Frames are immutable `Arc<BoxPlotFrame>` snapshots, holding their GPU resources.
A new model/size/scale/font generation produces a new frame; identical calls
return the same Arc. Surface styles, individual color, hover/selection and
background changes reuse the annotation atlas. Geometry/text/layout edits and
legend colors rebuild it. `shares_annotations_with` exposes this for diagnostics.
Changing a box color changes the box, not the series legend swatch.

The default separate GPU budget is 256 MiB. `set_memory_budget`,
`gpu_memory_usage`, `clear_cache`, and `end_frame` follow the small-chart APIs.
After submitting or discarding all recorded frames, call `end_frame` to schedule
retired resource destruction at queue completion. External snapshots keep their
resources alive. Native hosts must poll the device; browser callbacks complete
through the event loop. The host owns target textures and their budget separately.

`export_rgba_async(&chart, size, scale).await` works on native and WebGPU. Native
also has `export_rgba`. The result is straight-alpha RGBA; `encode_png` produces
PNG bytes. Export uses the current SSOT, including hover and selection: clear
those fields in a clone when a presentation should omit interaction emphasis.
RGBA8/BGRA8, linear/sRGB targets are supported. Render passes must use the format
chosen at renderer creation and sample count 1.

## Examples and tests

```bash
cargo run --locked -p figgy-renderer --example boxplot_gallery -- target/boxplot-gallery
cargo run --locked -p figgy-renderer --example boxplot_editor --features egui_demo
cargo test --locked -p figgy-model --features serde boxplot
cargo test --locked -p figgy-renderer --test boxplot_render
NO_HEADLESS=1 CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
  cargo test --locked -p figgy-renderer --test boxplot_browser --target wasm32-unknown-unknown
```

The browser runner must match the locked wasm-bindgen version; open its URL in
a WebGPU browser. Tests exercise the renderer directly, not a web Studio wrapper.
The native gallery produces five actual GPU PNGs: matte, satin, horizontal,
grouped notch, and mean/selection. All statistics are synthetic. The editor
supports component picking, summary tooltips, whole-chart and individual style
editing, color changes, resets, direction, grid, legend and category reordering.

## 한국어 사용법

박스플롯은 집계한 통계값을 받는다. `BoxSummary::new(수염 하한, Q1, 중앙값, Q3,
수염 상한)`으로 만들고, 평균·표본 수·이상치·중앙값 신뢰구간은 필요할 때 추가한다.
수염이 최솟값/최댓값인지, 1.5 IQR 안의 관측값인지 등은 호스트에서 결정한다.
렌더러는 원본 표본으로 통계를 계산하거나 수염 규칙을 추측하지 않는다.

노치를 켜려면 `median_ci`를 넣어야 한다. 신뢰구간이 상자 밖으로 나와도 잘라서
통계적 의미를 바꾸지 않는다. Q1과 Q3가 같으면 채운 상자는 없지만 중앙값과 수염 등은
설정에 따라 표시한다. `None`은 누락값이며, 그 범주·시리즈의 자리는 그대로 둔다.

가로형은 `direction = Horizontal`, 그룹형은 범주마다 여러 시리즈를 넣으면 된다.
각 상자의 색·재질·외곽선·노치·표기는 `BoxPlotOverride`로 바꾼다. 스타일은 전체
교체이므로 부모 스타일을 복제한 뒤 필요한 값만 바꾸면 된다. 개별 설정을 `None`으로
되돌리면 상위 설정을 따른다. 외곽선을 꺼도 중앙값과 수염은 별도 설정을 유지한다.

색이나 호버 강조를 바꿔도 통계값의 좌표는 움직이지 않는다. 기본 자동 범위는 숨긴
평균·이상치·신뢰구간까지 포함하므로 표시 옵션을 켜고 끄는 것만으로 축이 바뀌지 않는다.
직접 범위를 지정하면 영역 밖의 도형과 선택 판정을 함께 자른다. 글자가 겹치는 크기에서는
오류를 반환하므로 창을 넓히거나 글자·표기 항목을 줄여야 한다.

현재는 네이티브 예제와 WASM 렌더러 API까지 제공한다. 웹 Studio 편집 UI에는 아직
연결하지 않았다. 초기 입력 상한은 64범주·16시리즈·총 512항목, 이상치 4096개이며
상자 하나에는 이상치를 최대 128개 받는다. 대용량 스트리밍은 이 계약에 포함하지 않는다.
