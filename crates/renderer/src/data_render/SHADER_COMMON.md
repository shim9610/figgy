# Shader common definitions (SSoT)

> **This file is NOT compiled.** WGSL has no `import`/`include`, so common
> struct/function definitions are duplicated into each shader file. The
> **single source of truth** for those duplicates lives here.
>
공통 정의를 수정할 때는 다음 순서를 따른다.

1. 이 문서의 해당 코드 블록을 먼저 수정한다.
2. 아래 동기화 대상 셰이더에서 같은 블록을 모두 수정한다.
3. 대응하는 CPU 구조체인 `mod.rs::ScatterTransform` / `PrimitiveStyle`의 크기와 필드 순서도 확인한다.
4. `cargo check && cargo test`로 빌드와 테스트를 실행한다.

한 파일만 바꾸면 컴파일은 성공해도 GPU가 메모리를 잘못 해석할 수 있다. 일부 시리즈의 색·위치·축만 어긋나는 오류를 피하려면 공통 정의를 함께 갱신해야 한다.

---

## 동기화 대상 셰이더

아래 파일들의 공통 블록은 항상 정확히 일치해야 한다.

- `scatter_columnar.wgsl`
- `line_columnar.wgsl`
- `errorbar_columnar.wgsl`
- `bar_columnar.wgsl`
- `field_columnar.wgsl`
- `line_arc.wgsl`: 경로 길이 계산용. `Transform`, `maybe_log`, vec2 입력의 `data_to_ndc`만 공유하고 `Style`은 사용하지 않는다.
- `contour_anchor.wgsl`: 라벨 앵커 계산용. 화면 좌표에서 위치를 선택하므로 `line_arc.wgsl`과 같은 세 블록을 사용한다.
- `contour_label.wgsl`: 라벨 렌더링용. 앵커를 NDC에 배치하며 위 세 블록을 공유한다. 색과 배경은 아틀라스에 포함돼 있어 `Style`은 사용하지 않는다.

기존의 마칭 스퀘어 추적용 `gpu_contour.wgsl`과 버킷 방식의 `gpu_contour_anchor.wgsl`은 제거됐다. 현재는 `field_columnar.wgsl::fs_contour`가 격자에서 등고선을 직접 그리고, `contour_anchor.wgsl`이 뉴턴 투영으로 앵커를 구한다. 따라서 두 셰이더가 격자 샘플링 블록을 공유한다.
라벨 투영은 `anchor_seed` → `anchor_step` 4회 → `anchor_project`의 최종 샘플·검증 →
`anchor_select` 순서로 실행한다. 보정 횟수와 격자 샘플링 정의는 그대로이며, 컴파일러가
복잡한 격자 검색을 바깥 반복문 안에서 처리하지 않도록 dispatch를 나눴다. 중간 `axis-t`
좌표는 기존 후보 레코드의 `x`에 잠시 저장하고 최종 단계에서 데이터 좌표 쌍으로 덮어쓴다.
후보 버퍼는 선택 단계 전까지 외부에서 읽지 않으며 새 버퍼·바인딩은 추가하지 않는다.

기존 추적 셰이더는 데이터 좌표에서 계산해 `Transform`을 읽지 않았으므로 공통 정의 목록에 포함되지 않았고, 확대·이동과 무관하게 결과를 캐시했다.

`fullscreen_textured.wgsl`은 텍스처·샘플러용 별도 바인드 레이아웃을 사용하므로 이 문서의 공통 정의를 사용하지 않는다.

각 셰이더 안의 공통 블록은 다음 주석으로 감싸 두었다:

```wgsl
// ───── BEGIN common block (SHADER_COMMON.md) ─────
//        ...
// ───── END common block ─────
```

공통 블록의 정의는 이 문서와 바이트 단위로 같아야 한다.

---

## 1. `Transform` uniform — group 0, binding 0

`Transform`은 데이터 좌표를 NDC로 바꾸는 값, 로그축 여부, 픽셀과 NDC의 비율, 스케치·별자리 등 현재 스타일의 옵션을 전달한다. 크기는 **112바이트**다. `vec2<f32>` 8개와 `array<vec4<f32>, 3>` 하나로 구성되며, 배열은 바이트 오프셋 64에서 시작하고 원소 간 간격은 16바이트다.
점 반지름과 오차 막대 끝선 길이 같은 픽셀 크기는 `Style`에 저장한다. 셰이더는 `pixel_to_ndc`로 이를 NDC 크기로 환산한다.

<!-- shader-common: applies-to=scatter,line,errorbar,bar,field,arc,anchor,label -->
```wgsl
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    // Generic per-panel style parameter slots. Interpretation belongs to the
    // ACTIVE style's shader entries; the precise entries never read them.
    // sketch:        [0] = (amplitude_px, wavelength_px, seed(f32), 0)
    // milkyway:      [0] = (star_density, ribbon_width_px, ribbon_intensity,
    //                seed(f32)), [1] = (star_scale, spread_px, faint_bias, planet_rim),
    //                [2] = (structure_scale, star_brightness, 0, 0) — multiplier on the
    //                style's px-denominated structure constants (clump
    //                wavelength, binary separation); keeps the star texture
    //                resolution-invariant under DPI/export scaling.
    // constellation: [0] = (star_opacity, line_opacity, 0, 0)
    // All styles reserve [2].z for the global point-base u32 BIT PATTERN.
    // Resident draws write zero; streamed point/errorbar draws bitcast it.
    style_params: array<vec4<f32>, 3>,
};  // 112 B (vec4 array at offset 64, stride 16)

@group(0) @binding(0) var<uniform> transform: Transform;

fn styled_point_index(local_index: u32) -> u32 {
    return bitcast<u32>(transform.style_params[2].z) + local_index;
}
```

| 필드 | 의미 |
|------|------|
| `data_min`, `data_max` | `Config`에 지정된 축의 최소·최대를 셰이더 좌표계로 옮긴 `(hi, lo)` 범위. 선형축은 설정값을, 로그축은 CPU에서 계산한 log10 값을 쓴다. 데이터 영역 여백을 이유로 범위를 별도로 늘리지 않는다. |
| `scale_log` | 축별 로그 변환 여부. 0.0은 선형, 1.0은 log10 |
| `pixel_to_ndc` | `(2/chart_w, 2/chart_h)`. 픽셀 크기를 NDC 크기로 바꾸는 비율이며 선 두께·점 반지름·오차 막대 끝선 길이에 사용한다. |
| `data_to_panel_scale`, `data_to_panel_offset` | 축 범위로 정규화한 좌표를 패널 좌표로 바꾸는 데이터 영역의 아핀 변환. 축 범위와 배치 여백을 분리한다. |
| `style_params` | Generic style parameter slots. Interpretation belongs to the active styled entry. sketch: `[0]`=(amplitude_px, wavelength_px, seed(f32), 0), rest 0. milkyway: `[0]`=(star_density, ribbon_width_px, ribbon_intensity, seed(f32)), `[1]`=(star_scale, spread_px, faint_bias, planet_rim), `[2]`=(structure_scale, star_brightness, 0, 0). constellation: `[0]`=(star_opacity, line_opacity, 0, 0), rest 0. Seeds are stored as f32 and recovered as `u32(...)`; exact up to 2^24. CPU packing lives in renderer.rs (`StyleVariant::pack_params`, `[f32; 12]`). Precise entries do not read these slots. |

**대응하는 CPU 구조체:** `src/data_render/mod.rs::ScatterTransform`
(`#[repr(C)]`, `bytemuck::Pod`). 필드 순서와 크기가 정확히 일치해야 한다.

`style_params[2].z` is reserved independently of the active style: it carries
the exact **u32 bit pattern** of a streamed point/errorbar chunk's global base,
not a numeric f32 conversion. `styled_point_index` adds the local instance id
as an integer, so identities above 2^24 keep all bits. Resident transforms write
zero. `create_stream_point_transform_bind_group` clones only the 112-byte
transform metadata into a separately charged immutable uniform; it never
rewrites the shared view transform or copies source payloads. Callers validate
the global range before creating the uniform. Line arc/star-slot identities
are separate and do not consume this field.

---

## 2. `Style` uniform — group 1, binding 0

`Style`은 알파를 미리 곱한 색과 도형별 옵션을 전달한다. 크기는 **80바이트**, 정렬 단위는 **16바이트**다. 공유하는 셰이더들은 각자 필요한 필드만 읽는다.

<!-- shader-common: applies-to=scatter,line,errorbar,bar,field -->
```wgsl
struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    // Per-series decorrelation salt (FNV-1a of series_id). Styled entries
    // (sketch/milkyway/constellation) XOR it into their hash seeds so two series never
    // share a star/wobble pattern; precise entries never read it.
    series_salt: u32,
    // Primitive-specific feature bits. Errorbar uses bit 0 for Y and bit 1
    // for X; every other primitive ignores this field.
    primitive_flags: u32,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;
```

| 필드 | 의미 |
|------|------|
| `color_premul` | premultiplied RGBA. `(r·a, g·a, b·a, a)` |
| `line_width_px` | 선 또는 오차 막대 몸통의 두께(px) |
| `point_radius_px` | 산점도 기호의 반지름(px) |
| `cap_half_px` | 오차 막대 끝선 길이의 절반(px) |
| `cap_width_px` | 오차 막대 끝선의 두께(px) |
| `shape_id` | `ScatterShape` GPU 코드 — 0 Circle, 1 Square, 2 Triangle, 3 Diamond, 4 Cross, 5 CircleFilled, 6 SquareFilled, 7 TriangleFilled, 8 DiamondFilled, 9 TriangleDown, 10 TriangleLeft, 11 TriangleRight, 12 Plus, 13 Pentagon, 14 Hexagon, 15 Octagon, 16 Star, 17 TriangleDownFilled, 18 TriangleLeftFilled, 19 TriangleRightFilled, 20 PlusFilled, 21 CrossFilled, 22 PentagonFilled, 23 HexagonFilled, 24 OctagonFilled, 25 StarFilled |
| `dash_len` | `dash`의 유효 스칼라 개수. 0은 실선 |
| `series_salt` | 시리즈마다 해시 패턴을 다르게 만드는 값. `renderer.rs`의 `create_style_for_series*`가 `fnv1a(series_id)`를 기록한다. 스케치·별자리 셰이더는 이 값을 시드에 XOR해 같은 X좌표를 쓰는 시리즈에서도 선의 흔들림과 별 패턴이 겹치지 않게 한다. 정밀 모드에서는 읽지 않는다. |
| `primitive_flags` | 도형별 기능 비트. 오차 막대는 bit 0으로 Y방향, bit 1로 X방향 사용 여부를 표시한다. 다른 도형은 읽지 않는다. 기존 패딩을 사용하므로 크기는 80바이트로 유지된다. |
| `dash` | 최대 8개의 순차 `[on, off, ...]` 픽셀 길이 — `dash[0].xyzw` 먼저, 이어서 `dash[1].xyzw` |

**대응하는 CPU 구조체:** `src/data_render/mod.rs::PrimitiveStyle`
(`#[repr(C)]`, `bytemuck::Pod`). 패딩 포함 80바이트. `shape_id` 매핑은
`mod.rs::shape_id()` 헬퍼가 담당한다.

### 2.1 `bar_columnar.wgsl`의 `Style` 재해석

막대는 공통 `Style` 구조체의 크기와 배치를 유지하면서 일부 필드를 다른 용도로 사용한다. 아래 표가 그 해석 기준이며 CPU의 `mod.rs::PrimitiveStyle::from_bar`와 일치해야 한다. 한쪽만 바꾸면 색·두께·기준선이 잘못 전달될 수 있다.

| 필드 | 막대에서의 의미 |
|------|------------------|
| `color_premul` | 알파를 미리 곱한 채움색 |
| `line_width_px` | 테두리 두께(픽셀) |
| `cap_half_px` | 이웃 막대 사이 간격(픽셀). 양쪽에 절반씩 |
| `cap_width_px` | 구간 너비에서 막대가 차지하는 비율. `0..=1`로 제한하고 가운데에 배치 |
| `shape_id` | 0 = 경계가 X축을 따른다(세로 막대), 1 = Y축을 따른다(가로 막대) |
| `dash[0]` | 알파를 미리 곱한 테두리색 |
| `dash[1].xy` | 기준선을 풀과 같은 f32 쌍 `(hi, lo)`으로 저장 |
| `point_radius_px` · `dash_len` · `primitive_flags` · `dash[1].zw` | 미사용 |

막대는 점선 패턴을 사용하지 않으므로 `dash`의 f32 슬롯 8개를 재사용한다. 기준선은 풀의 값과 같은 `(hi, lo)` 쌍으로 저장해 큰 값에서도 정밀도를 유지한다. f32 하나로 저장하면 이 정밀도를 보존할 수 없다.

`DataBarStyleConfig.bar_style_overrides`를 지정하면 개별 막대 스타일용 파이프라인이 group 2의 덮어쓰기 표를 읽는다. 채움색·테두리색·두께·간격·너비 비율만 바꾸며 기준선과 방향은 시리즈 공통값을 유지한다. 그리기·데이터 피킹·선택 테두리는 같은 해석 규칙을 적용한다.

### 2.2 `field_columnar.wgsl`의 `Style` 사용

히트맵과 밴드 채움은 `color_premul`만 읽는다. 이 값은 알파를 미리 곱한 `Config.colorbar.nan_color`로, NaN·로그 스케일의 0 이하 값·너비가 없는 범위를 표시하는 색이다. `primitive_flags`를 포함한 다른 필드는 읽지 않는다.

나머지 행렬 옵션은 별도 유니폼인 `FieldParams`(group 2, binding 4)에 저장한다. 좌표 컬럼의 시작 위치, 격자 크기·방향, Z축 범위, 레벨·색상 기준점 개수는 `Style`의 빈 필드에 모두 담을 수 없기 때문이다.

group 2의 격자 스토리지와 유니폼은 행렬 렌더링과 등고선 앵커 계산이 공유하므로 동기화 대상이다. 대응하는 CPU 구조체는 `mod.rs::FieldParamsGpu` / `GridColumnGpu`다. 검색용 메타데이터인 binding 6은 `field_columnar.wgsl`에만 선언하며, 아래 5.1절의 규칙을 따른다.

---

<a id="3-maybe_log--log10-인입통과-헬퍼"></a>

## 3. `maybe_log` — 로그 변환 선택

`is_log` 플래그(0.0 또는 1.0)에 따라 값을 그대로 통과시키거나 log10을
적용한다. `if` 분기 없이 `mix`로 처리해 워프 단위 분기 비용을 피한다.

<!-- shader-common: applies-to=scatter,line,errorbar,bar,field,arc,anchor,label -->
```wgsl
fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}
```

호출자는 `transform.scale_log.x` 또는 `transform.scale_log.y`를 `is_log`로
넣어 X·Y축을 독립적으로 선택한다.

---

## 4. `axis_pair_to_t` / `data_to_ndc` — hi/lo 데이터 좌표 → NDC

Column pool logical values are `vec2<f32>(hi, lo)`. Linear axes compute the
range-local numerator as `(value_hi - min_hi) + (value_lo - min_lo)` so large
absolute timestamps still preserve small deltas after upload. Log axes use the
recombined display value (`hi + lo`) before comparing against the log-transformed
axis bounds.

<!-- shader-common: applies-to=scatter,line,errorbar,bar,field,arc,anchor,label -->
```wgsl
fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = (v.x - min_hi) + (v.y - min_lo);
    let range = (max_hi - min_hi) + (max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    let data_t = mix(linear_num / range, log_num / range, is_log);
    return panel_offset + data_t * panel_scale;
}

fn data_to_ndc(xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    let tx = axis_pair_to_t(xv, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, transform.data_to_panel_scale.x, transform.data_to_panel_offset.x);
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
}
```

---

---

## 5. 격자 샘플링 — group 2 (면·등고선)

이 블록은 행렬의 Z값을 읽고 셀 내부의 이중선형 보간값과 해석적 기울기를 계산한다. 채움·등고선 렌더링(`field_columnar.wgsl`의 `fs_main` / `fs_contour`)과 라벨 앵커 계산(`contour_anchor.wgsl`의 뉴턴 투영)이 같은 바인드 그룹과 함수를 사용해야 한다. 서로 다르면 라벨이 등고선에서 벗어날 수 있다.

`locate`의 `lattice` 인자는 검색할 격자를 정한다. 채움은 `Shading`에 따른 사각형 격자를, 등고선은 실제 Z값이 있는 표본 격자를 사용한다. 두 경우 모두 같은 이진 탐색을 수행한다.

`level_colors`(binding 5)는 앵커 셰이더에서 읽지 않지만 바인드 그룹 배치를 맞추기 위해 공통 블록에 둔다. WGSL은 사용하지 않는 전역 선언을 허용한다.

대응하는 CPU 정의는 `mod.rs::GridColumnGpu`(8바이트), `mod.rs::FieldParamsGpu`(64바이트), `create_field_data_bind_group_layout`이다.

`gpu_errorbar.wgsl`의 행렬 범위 계산은 전체 샘플링 블록 대신 `*_grid_pair` 연산 함수만 공유한다. `shader_consistency`가 함수 본문을 이 문서와 바이트 단위로 비교하므로 GPU 범위 계산과 실제 그리기가 같은 반올림 규칙을 사용한다.

<!-- shader-common: applies-to=field,anchor -->
```wgsl
// ── Field grid sampling (group 2) ───────────────────────────────────────────
//
// No textures and no vertex buffers: everything is read from storage. The pool
// is bound whole, with the coordinate columns located by `FieldParams` lane
// bases, exactly as the constellation star pass does it.

/// One constituent grid column: where it starts in the pool (as an f32 lane
/// index) and how many logical values it holds.
struct GridColumn {
    base: u32,
    len: u32,
};

/// Bit flags in `FieldParams.flags`.
const FIELD_COLUMNS_ARE_Y: u32 = 1u;
const FIELD_CENTERS: u32 = 2u;
const FIELD_INTERPOLATED: u32 = 4u;
const FIELD_BANDS: u32 = 8u;
const FIELD_LOG_Z: u32 = 16u;

/// Everything the field needs that is not already in `Transform` / `Style`.
///
/// `z_min` / `z_max` arrive **already log-transformed** when `FIELD_LOG_Z` is
/// set: the colourbar's bounds are f64 on the host, so taking their logarithm
/// there keeps the precision that doing it here would lose. Both are `(hi, lo)`
/// f32 pairs, the pool's own representation.
///
/// CPU twin: `mod.rs::FieldParamsGpu` (`#[repr(C)]`, 64 B).
struct FieldParams {
    x_base: u32,
    y_base: u32,
    x_len: u32,
    y_len: u32,
    cols: u32,
    rows: u32,
    level_count: u32,
    stop_count: u32,
    flags: u32,
    opacity: f32,
    /// Contour stroke width in pixels, already multiplied by the render scale.
    /// Read by `fs_contour`; the fill ignores it.
    line_width_px: f32,
    /// Entries in `level_colors`. A level past the end takes the last one.
    level_color_count: u32,
    z_min: vec2<f32>,
    z_max: vec2<f32>,
};

@group(2) @binding(0) var<storage, read> field_pool: array<f32>;
@group(2) @binding(1) var<storage, read> grid: array<GridColumn>;
@group(2) @binding(2) var<storage, read> levels: array<f32>;
/// The first `max(field.stop_count, 1)` entries are colourmap stops or the
/// required empty-table padding. The remaining entries preserve one record per
/// declared contour level. Each 32-record block starts with its finite keys,
/// sorted by x; y is the original declaration index as a normal numeric f32.
/// Non-finite slots are zero padding and binding 6 carries the searchable prefix
/// length. Binding 2 remains the positional SSoT.
@group(2) @binding(3) var<storage, read> stops: array<vec4<f32>>;
@group(2) @binding(4) var<uniform> field: FieldParams;
/// Per-level contour stroke colour, **premultiplied**. One entry per level; the
/// fill never reads it.
@group(2) @binding(5) var<storage, read> level_colors: array<vec4<f32>>;

fn field_flag(bit: u32) -> bool {
    return (field.flags & bit) != 0u;
}

fn pool_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(field_pool[j], field_pool[j + 1u]);
}

fn grid_rounded_add(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a + b));
}

fn grid_rounded_subtract(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a - b));
}

/// Normalize a finite pair with error-free addition. The bitcasts expose every
/// f32 rounding boundary so field rendering and fit reduction cannot be
/// reassociated differently by their backends.
fn normalize_grid_pair(v: vec2<f32>) -> vec2<f32> {
    let sum = grid_rounded_add(v.x, v.y);
    let b_virtual = grid_rounded_subtract(sum, v.x);
    let a_virtual = grid_rounded_subtract(sum, b_virtual);
    let b_roundoff = grid_rounded_subtract(v.y, b_virtual);
    let a_roundoff = grid_rounded_subtract(v.x, a_virtual);
    return vec2<f32>(sum, grid_rounded_add(a_roundoff, b_roundoff));
}

fn add_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let high = normalize_grid_pair(vec2<f32>(a.x, b.x));
    let low = grid_rounded_add(grid_rounded_add(a.y, b.y), high.y);
    return normalize_grid_pair(vec2<f32>(high.x, low));
}

fn subtract_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return add_grid_pairs(a, -b);
}

fn scale_grid_pair(v: vec2<f32>, factor: f32) -> vec2<f32> {
    return normalize_grid_pair(vec2<f32>(v.x * factor, v.y * factor));
}

fn midpoint_grid_pair(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    // Scale first so two same-sign maximum f32 coordinates can still have a
    // finite midpoint.
    return add_grid_pairs(scale_grid_pair(a, 0.5), scale_grid_pair(b, 0.5));
}

/// `axis_pair_to_t` for whichever axis: 0 = x, 1 = y.
fn axis_t(pair: vec2<f32>, axis: u32) -> f32 {
    if (axis == 0u) {
        return axis_pair_to_t(pair, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, transform.data_to_panel_scale.x, transform.data_to_panel_offset.x);
    }
    return axis_pair_to_t(pair, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
}

/// Cell boundary `k` along one axis, in pool `(hi, lo)` pair form.
///
/// `Edges`: boundary k *is* coordinate k. `Centers`: boundary k is the midpoint
/// of coordinates k-1 and k, with the two outer boundaries mirrored out by the
/// adjacent half-cell. With one coordinate every boundary collapses onto it.
fn cell_edge_pair(base: u32, n: u32, k: u32) -> vec2<f32> {
    if (!field_flag(FIELD_CENTERS)) {
        return pool_pair(base, k);
    }
    if (k == 0u) {
        let c0 = pool_pair(base, 0u);
        let c1 = pool_pair(base, min(1u, n - 1u));
        return add_grid_pairs(c0, scale_grid_pair(subtract_grid_pairs(c0, c1), 0.5));
    }
    if (k >= n) {
        let last = pool_pair(base, n - 1u);
        let prev = pool_pair(base, max(n, 2u) - 2u);
        return add_grid_pairs(last, scale_grid_pair(subtract_grid_pairs(last, prev), 0.5));
    }
    return midpoint_grid_pair(pool_pair(base, k - 1u), pool_pair(base, k));
}

/// Sample point `i` along one axis — where a z value sits, as opposed to where
/// its cell ends. `Centers` samples are the coordinates themselves; `Edges`
/// samples sit at the middle of each cell.
fn sample_point_pair(base: u32, n: u32, i: u32) -> vec2<f32> {
    if (field_flag(FIELD_CENTERS)) {
        return pool_pair(base, i);
    }
    return midpoint_grid_pair(pool_pair(base, i), pool_pair(base, min(i + 1u, n - 1u)));
}

/// Which lattice a lookup walks.
///
/// Two different questions share one binary search. The **fill** asks about the
/// quads it draws, which follow `Shading`: cell-to-cell when flat, point-to-point
/// when interpolated. A **contour** always asks about the sample points, because
/// that is where z lives — an isoline does not move because the fill decided to
/// draw flat cells.
const LATTICE_QUADS: u32 = 0u;
const LATTICE_SAMPLES: u32 = 1u;

fn lattice_is_samples(lattice: u32) -> bool {
    return lattice == LATTICE_SAMPLES || field_flag(FIELD_INTERPOLATED);
}

/// Boundary `k` of one lattice along one axis.
fn boundary_t(base: u32, n: u32, k: u32, axis: u32, lattice: u32) -> f32 {
    if (lattice_is_samples(lattice)) {
        return axis_t(sample_point_pair(base, n, k), axis);
    }
    return axis_t(cell_edge_pair(base, n, k), axis);
}

/// Quads along one axis: one per cell on the quad lattice, one per gap between
/// sample points on the sample lattice.
fn quad_count(cells: u32, lattice: u32) -> u32 {
    if (lattice_is_samples(lattice)) {
        return max(cells, 1u) - 1u;
    }
    return cells;
}

/// Where a fragment landed along one axis.
struct CellHit {
    /// Quad index along this axis. Meaningless unless `hit`.
    index: u32,
    /// Position inside that quad, 0 at its low boundary, 1 at its high one.
    frac: f32,
    hit: bool,
};

/// Which quad along one axis contains `t`, and where inside it.
///
/// Boundaries are monotone in `t` but may descend (inverted axis), so the
/// direction is read off the two ends rather than assumed. The loop is
/// `O(log count)` — it stops as soon as the bracket is one wide.
fn locate(base: u32, n: u32, count: u32, axis: u32, t: f32, lattice: u32) -> CellHit {
    var out: CellHit;
    out.index = 0u;
    out.frac = 0.0;
    out.hit = false;
    if (count == 0u || n == 0u || !f32_is_finite(t)) {
        return out;
    }
    let first = boundary_t(base, n, 0u, axis, lattice);
    let last = boundary_t(base, n, count, axis, lattice);
    if (!f32_is_finite(first) || !f32_is_finite(last)) {
        return out;
    }
    if (t < min(first, last) || t > max(first, last)) {
        return out;
    }
    let ascending = last >= first;
    var lo = 0u;
    var hi = count;
    // Each step strictly shrinks the unsigned bracket, so a u32 count takes
    // at most 32 halvings. Keep the loop dynamic: nested fixed-trip searches
    // can make software GPU compilers expand contour projection excessively.
    while (hi - lo > 1u) {
        let mid = lo + (hi - lo) / 2u;
        let tm = boundary_t(base, n, mid, axis, lattice);
        if (!f32_is_finite(tm)) {
            return out;
        }
        let before = select(tm > t, tm <= t, ascending);
        if (before) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let a = boundary_t(base, n, lo, axis, lattice);
    let b = boundary_t(base, n, lo + 1u, axis, lattice);
    let span = b - a;
    if (!f32_is_finite(a) || !f32_is_finite(b) || !f32_is_finite(span)) {
        return out;
    }
    out.index = lo;
    // A zero-width quad (a lone centre) has no inside; treat it as its start.
    var frac = 0.0;
    if (span != 0.0) {
        frac = (t - a) / span;
        if (!f32_is_finite(frac)) {
            return out;
        }
    }
    out.frac = clamp(frac, 0.0, 1.0);
    out.hit = true;
    return out;
}

struct GridValue {
    pair: vec2<f32>,
    valid: bool,
};

/// The grid's z at constituent column `c`, value `r`, as a pool pair plus
/// explicit bounds validity. The pair remains zero when the address is invalid.
fn grid_value(c: u32, r: u32) -> GridValue {
    var out: GridValue;
    out.pair = vec2<f32>(0.0, 0.0);
    out.valid = false;
    if (c >= field.cols) {
        return out;
    }
    let column = grid[c];
    if (r >= column.len) {
        return out;
    }
    let j = column.base + r * 2u;
    out.pair = vec2<f32>(field_pool[j], field_pool[j + 1u]);
    out.valid = true;
    return out;
}

/// Whether `v` is finite, decided from its bits.
///
/// WGSL does not guarantee IEEE NaN semantics, so `v != v` is not a NaN test —
/// a backend may lower it to an *ordered* comparison, which answers `false` for
/// NaN and lets a missing value be painted as the ramp's low end. The exponent
/// field is unambiguous: all ones means NaN or an infinity, and both are
/// unplaceable.
fn f32_is_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

fn vec2_f32_is_finite(v: vec2<f32>) -> bool {
    return f32_is_finite(v.x) && f32_is_finite(v.y);
}

/// z and its gradient at an axis-`t` point, on the **sample** lattice.
///
/// The one place a contour's geometry comes from. There is no segment list and
/// no marching-squares case table: z is a bilinear function inside each cell, so
/// its gradient is available in closed form and the isoline is `z == level`
/// exactly. A saddle is a hyperbola and needs no tie-break rule.
///
/// `dpdx`/`dpdy` are deliberately not used. Those are 2x2 quad finite
/// differences, so a quad straddling a cell boundary reports a gradient that
/// belongs to neither cell — one quad of wrong stroke width along every
/// boundary. The analytic derivative is exact within the cell.
struct ContourSample {
    hit: bool,
    z: f32,
    /// dz/dt in axis-`t` units, i.e. per unit of chart area.
    dz: vec2<f32>,
    /// The only non-zero second derivative a bilinear cell has: the cross term
    /// `d2z/dtx dty`. Both pure second derivatives are identically zero, so this
    /// one number *is* the Hessian, exactly — no finite difference involved.
    d2: f32,
    /// The cell's z range. A bilinear surface attains its extremes at the
    /// corners, so this bounds z over the whole cell and lets a level be
    /// rejected without evaluating anything.
    z_lo: f32,
    z_hi: f32,
};

fn contour_sample(t: vec2<f32>) -> ContourSample {
    var out: ContourSample;
    out.hit = false;
    out.z = 0.0;
    out.dz = vec2<f32>(0.0, 0.0);
    out.d2 = 0.0;
    out.z_lo = 0.0;
    out.z_hi = 0.0;
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(field.x_base, field.x_len, quad_count(along_cells, LATTICE_SAMPLES), 0u, t.x, LATTICE_SAMPLES);
    let y = locate(field.y_base, field.y_len, quad_count(across_cells, LATTICE_SAMPLES), 1u, t.y, LATTICE_SAMPLES);
    if (!x.hit || !y.hit) {
        return out;
    }
    let c = select(x.index, y.index, columns_are_y);
    let r = select(y.index, x.index, columns_are_y);
    let p00 = grid_value(c, r);
    let p10 = grid_value(c + 1u, r);
    let p01 = grid_value(c, r + 1u);
    let p11 = grid_value(c + 1u, r + 1u);
    // One missing corner makes the whole cell unplaceable — a contour through
    // invented data is worse than a gap.
    if (!p00.valid || !p10.valid || !p01.valid || !p11.valid) {
        return out;
    }
    // Bounds validity and stored-value finiteness are separate facts. Inspect
    // both pair lanes by bits before arithmetic so actual NaN/Infinity remains
    // a contour gap on backends that do not preserve IEEE NaN comparisons.
    if (!vec2_f32_is_finite(p00.pair) || !vec2_f32_is_finite(p10.pair)
        || !vec2_f32_is_finite(p01.pair) || !vec2_f32_is_finite(p11.pair)) {
        return out;
    }
    let z00 = p00.pair.x + p00.pair.y;
    let z10 = p10.pair.x + p10.pair.y;
    let z01 = p01.pair.x + p01.pair.y;
    let z11 = p11.pair.x + p11.pair.y;
    if (!f32_is_finite(z00) || !f32_is_finite(z10)
        || !f32_is_finite(z01) || !f32_is_finite(z11)) {
        return out;
    }
    let z_lo = min(min(z00, z10), min(z01, z11));
    let z_hi = max(max(z00, z10), max(z01, z11));
    // `u` runs along the constituent axis (the one the columns are laid out on),
    // `v` within a column — the same convention `grid_value(c, r)` uses.
    let u = select(x.frac, y.frac, columns_are_y);
    let v = select(y.frac, x.frac, columns_are_y);
    if (!f32_is_finite(u) || !f32_is_finite(v)) {
        return out;
    }
    let du0 = z10 - z00;
    let du1 = z11 - z01;
    let dv0 = z01 - z00;
    let dv1 = z11 - z10;
    if (!f32_is_finite(du0) || !f32_is_finite(du1)
        || !f32_is_finite(dv0) || !f32_is_finite(dv1)) {
        return out;
    }
    let low = z00 + du0 * u;
    let high = z01 + du1 * u;
    if (!f32_is_finite(low) || !f32_is_finite(high)) {
        return out;
    }
    let z_delta = high - low;
    let z = low + z_delta * v;
    let dz_du = du0 * (1.0 - v) + du1 * v;
    let dz_dv = dv0 * (1.0 - u) + dv1 * u;
    let d2_num = du1 - du0;
    if (!f32_is_finite(z_delta) || !f32_is_finite(z)
        || !f32_is_finite(dz_du) || !f32_is_finite(dz_dv)
        || !f32_is_finite(d2_num)) {
        return out;
    }
    // (u, v) are cell fractions; the chain rule needs the cell's span in axis
    // `t`. The span may be negative on an inverted axis, which flips the
    // gradient's sign exactly as it should.
    let x_span = boundary_t(field.x_base, field.x_len, x.index + 1u, 0u, LATTICE_SAMPLES)
        - boundary_t(field.x_base, field.x_len, x.index, 0u, LATTICE_SAMPLES);
    let y_span = boundary_t(field.y_base, field.y_len, y.index + 1u, 1u, LATTICE_SAMPLES)
        - boundary_t(field.y_base, field.y_len, y.index, 1u, LATTICE_SAMPLES);
    let u_span = select(x_span, y_span, columns_are_y);
    let v_span = select(y_span, x_span, columns_are_y);
    if (!f32_is_finite(u_span) || !f32_is_finite(v_span)
        || u_span == 0.0 || v_span == 0.0) {
        // A zero-width cell (a lone coordinate) has no interior to place a
        // contour in, the same answer the fill gives it.
        return out;
    }
    let dz_dtu = dz_du / u_span;
    let dz_dtv = dz_dv / v_span;
    // The cross term is symmetric in the two axes, so it needs no orientation
    // select: swapping u and v leaves it unchanged.
    let span_product = u_span * v_span;
    if (!f32_is_finite(span_product) || span_product == 0.0) {
        return out;
    }
    let d2 = d2_num / span_product;
    let dz = select(
        vec2<f32>(dz_dtu, dz_dtv),
        vec2<f32>(dz_dtv, dz_dtu),
        columns_are_y,
    );
    if (!vec2_f32_is_finite(dz) || !f32_is_finite(d2)) {
        return out;
    }
    out.z = z;
    out.dz = dz;
    out.d2 = d2;
    out.z_lo = z_lo;
    out.z_hi = z_hi;
    out.hit = true;
    return out;
}

/// An axis-`t` gradient in **pixels**: z units per pixel on each screen axis.
///
/// `t` spans the chart area, NDC spans 2 across it, and `pixel_to_ndc` is one
/// pixel in NDC — so one pixel is `pixel_to_ndc / 2` of `t`.
fn grad_px(dz: vec2<f32>) -> vec2<f32> {
    return dz * transform.pixel_to_ndc * 0.5;
}
```

<a id="51-field-only-contour-lookup-metadata--group-2-binding-6"></a>

### 5.1 행렬 셰이더의 등고선 검색 메타데이터 — group 2, binding 6

`field_columnar.wgsl`은 공통 블록 밖에서 binding 6을 선언한다. 레벨 32개짜리 블록마다 `[finite_count, negative_infinity_count]`라는 u32 두 개, 총 8바이트를 기록한다.
`finite_count`는 binding 3 블록 앞부분에 정렬해 둔 유한 레벨 수이며 선·밴드 이진 탐색의 범위를 정한다. `negative_infinity_count`는 밴드 계산의 분자에만 더하고 선 후보에는 포함하지 않는다. NaN과 양의 무한대는 검색과 분자에서 모두 제외한다. 레벨이 없어도 WebGPU의 크기 0인 스토리지 바인딩을 피하려고 `[0, 0]` 하나를 업로드한다.

CPU 구조체는 `mod.rs::ContourLookupMetadataGpu`(8바이트)이며 `create_field_data_bind_group_layout`의 프래그먼트 전용 binding 6에 연결된다. 버퍼는 `FieldTables`에서 다른 group 2 표와 함께 만들고 바인드 그룹이 보관한다. 할당량은 같은 `ChargeTally`의 `FieldTable` 항목에 실제 바이트 수로 합산한다.

---
## 변경 체크리스트

공통 정의를 변경했다면 다음 항목을 모두 확인한다.

- [ ] 본 문서의 해당 블록을 먼저 수정했다.
- [ ] `scatter_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `line_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `errorbar_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `bar_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `field_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `line_arc.wgsl`의 공통 블록을 수정했다 (Transform/maybe_log/
      data_to_ndc(vec2) 해당 시).
- [ ] `contour_anchor.wgsl`의 공통 블록을 수정했다 (Transform 3블록 + 격자 샘플링 블록).
- [ ] `contour_label.wgsl`의 공통 블록을 수정했다 (같은 3블록).
- [ ] `mod.rs::ScatterTransform` / `PrimitiveStyle`의 필드와 바이트 크기를
      확인했다 (struct 크기가 바뀌었다면 `expected_size` 검증문도 갱신).
- [ ] `cargo check` 통과.
- [ ] `cargo test` 통과 (특히 파이프라인 컴파일 테스트).
