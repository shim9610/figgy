# Shader common definitions (SSoT)

> **This file is NOT compiled.** WGSL has no `import`/`include`, so common
> struct/function definitions are duplicated into each shader file. The
> **single source of truth** for those duplicates lives here.
>
> **수정 절차 (반드시 이 순서):**
>
> 1. 이 문서를 먼저 수정한다.
> 2. 아래 *동기화 대상 셰이더* 목록의 **모든** 파일에서 해당 블록을
>    동일하게 수정한다. 한 파일만 고치고 끝내지 않는다.
> 3. CPU 측 짝(`mod.rs::ScatterTransform` / `PrimitiveStyle`)도 함께
>    확인·수정한다. 크기·필드 순서가 어긋나면 GPU 메모리가 silent하게
>    오역된다.
> 4. `cargo check && cargo test`로 빌드/테스트가 통과하는지 확인한다.
>
> 한 군데만 수정하면 다른 셰이더는 silent하게 어긋난 채 컴파일되며,
> 결과는 "왠지 모르게 색·위치·축이 일부 시리즈만 깨지는" 추적하기 매우
> 어려운 렌더링 버그가 된다.

---

## 동기화 대상 셰이더

이 파일들의 *common block* 은 항상 정확히 일치해야 한다:

- `scatter_columnar.wgsl`
- `line_columnar.wgsl`
- `errorbar_columnar.wgsl`
- `bar_columnar.wgsl`
- `field_columnar.wgsl`
- `line_arc.wgsl` (컴퓨트 — Transform / `maybe_log` / `data_to_ndc`(vec2)만
  공유, `Style` 은 사용하지 않음)
- `contour_anchor.wgsl` (컴퓨트 — 앵커 선택은 **화면** 공간이라 Transform이
  필요하다. `Style` 은 사용하지 않음 → `line_arc.wgsl` 과 같은 3블록)
- `contour_label.wgsl` (렌더 — 앵커를 NDC에 놓는다. 색·배경은 아틀라스에
  구워져 있어 `Style` 은 사용하지 않음 → 같은 3블록)

`gpu_contour.wgsl`(마칭스퀘어 추적)과 `gpu_contour_anchor.wgsl`(버킷 방식 앵커)은 §B.4.9에서
**삭제됐다**. 등고선은 이제 `field_columnar.wgsl::fs_contour`가 격자에서 직접 그리고, 앵커는
`contour_anchor.wgsl`이 같은 격자에서 Newton 투영으로 잡는다 — 그래서 앵커 셰이더가 격자 샘플링 블록을
공유한다.

삭제 전에도 `gpu_contour.wgsl`(추적)은 이 목록에 **없었다**: 마칭스퀘어는
데이터 공간이라 `Transform` 을 읽지 않았고, 그래서 팬·줌에 무관하게 캐시됐다.
공간이 다르면 파일도 달랐다.

`fullscreen_textured.wgsl`은 별도의 bind layout(texture/sampler)을 쓰므로
이 SSoT의 영향을 받지 않는다.

각 셰이더 안의 공통 블록은 다음 주석으로 감싸 두었다:

```wgsl
// ───── BEGIN common block (SHADER_COMMON.md) ─────
//        ...
// ───── END common block ─────
```

그 안의 모든 정의는 본 문서의 정의와 글자 단위로 동일해야 한다.

---

## 1. `Transform` uniform — group 0, binding 0

데이터 좌표 → NDC 변환, 로그 축 플래그, 픽셀↔NDC 환산 비율, 그리고
활성 렌더 스타일(스케치/성좌 등)의 범용 파라미터 슬롯을 셰이더에 전달하는
유니폼. **112바이트** (`vec2<f32>` 8개 + `array<vec4<f32>, 3>` 1개 —
배열은 offset 64, 원소 stride 16, WGSL uniform layout). 픽셀 단위
크기(점 반지름, cap 길이 등)는 `Style`로 이동했다 — 픽셀→NDC 환산은
셰이더가 `pixel_to_ndc`로 직접 수행한다.

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
| `data_min`, `data_max` | `Config` 축 SSoT의 최종 min/max를 셰이더 좌표계로 옮긴 `(hi, lo)` 범위. 선형축은 SSoT 값 자체이고 로그축은 CPU에서 계산한 `log10(SSoT)`다. data-area 여백 때문에 별도 확장 범위를 만들지 않는다. |
| `scale_log` | per-axis 로그 플래그. 0.0 = linear, 1.0 = log10 |
| `pixel_to_ndc` | `(2/chart_w, 2/chart_h)` — 1픽셀이 NDC에서 몇인지. 픽셀 단위 크기(line 두께, 점 반지름, cap 길이) 환산에 쓰임 |
| `data_to_panel_scale`, `data_to_panel_offset` | SSoT 범위 안의 정규화 좌표를 panel 좌표로 옮기는 data-area 배치 affine. 축 범위와 레이아웃 여백을 분리한다. |
| `style_params` | Generic style parameter slots. Interpretation belongs to the active styled entry. sketch: `[0]`=(amplitude_px, wavelength_px, seed(f32), 0), rest 0. milkyway: `[0]`=(star_density, ribbon_width_px, ribbon_intensity, seed(f32)), `[1]`=(star_scale, spread_px, faint_bias, planet_rim), `[2]`=(structure_scale, star_brightness, 0, 0). constellation: `[0]`=(star_opacity, line_opacity, 0, 0), rest 0. Seeds are stored as f32 and recovered as `u32(...)`; exact up to 2^24. CPU packing lives in renderer.rs (`StyleVariant::pack_params`, `[f32; 12]`). Precise entries do not read these slots. |

**CPU 측 짝:** `src/data_render/mod.rs::ScatterTransform`
(`#[repr(C)]`, `bytemuck::Pod`). 필드 순서·크기 1:1 일치해야 한다.

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

색(premultiplied alpha)과 per-primitive 옵션. **80바이트, 16바이트 정렬.**
네 셰이더가 같은 struct를 공유하고 각자 자기 필드만 읽는다(나머지는 무시).

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
| `line_width_px` | line / errorbar 스템 두께(픽셀) |
| `point_radius_px` | scatter 점 반지름(픽셀) |
| `cap_half_px` | errorbar cap 반-길이(픽셀) |
| `cap_width_px` | errorbar cap 스트로크 두께(픽셀) |
| `shape_id` | `ScatterShape` GPU 코드 — 0 Circle, 1 Square, 2 Triangle, 3 Diamond, 4 Cross, 5 CircleFilled, 6 SquareFilled, 7 TriangleFilled, 8 DiamondFilled, 9 TriangleDown, 10 TriangleLeft, 11 TriangleRight, 12 Plus, 13 Pentagon, 14 Hexagon, 15 Octagon, 16 Star, 17 TriangleDownFilled, 18 TriangleLeftFilled, 19 TriangleRightFilled, 20 PlusFilled, 21 CrossFilled, 22 PentagonFilled, 23 HexagonFilled, 24 OctagonFilled, 25 StarFilled |
| `dash_len` | `dash`의 유효 스칼라 개수. 0 = solid |
| `series_salt` | 시리즈 간 해시 탈상관 솔트 — `fnv1a(series_id)` (renderer.rs `create_style_for_series*`가 기록). 스케치/성좌 entry가 자기 해시 시드에 XOR한다. 같은 x 격자를 쓰는 시리즈들이 wobble/별 패턴을 공유하지 않게 하는 장치. 정밀 entry는 읽지 않음 |
| `primitive_flags` | primitive별 기능 비트. errorbar는 bit 0=`Y direction present`, bit 1=`X direction present`; 나머지 primitive는 무시. 기존 패딩 슬롯을 사용하므로 레이아웃은 80바이트로 동일 |
| `dash` | 최대 8개의 순차 `[on, off, ...]` 픽셀 길이 — `dash[0].xyzw` 먼저, 이어서 `dash[1].xyzw` |

**CPU 측 짝:** `src/data_render/mod.rs::PrimitiveStyle`
(`#[repr(C)]`, `bytemuck::Pod`). 패딩 포함 80바이트. `shape_id` 매핑은
`mod.rs::shape_id()` 헬퍼가 담당한다.

### 2.1 `bar_columnar.wgsl`의 `Style` 재해석

막대는 이 struct의 **바이트를 바꾸지 않고**(위 공통 `Style` 레이아웃 계약) 자기가 쓸 필드만
읽고 쓸모없는 필드를 재해석한다. 이 표가 그 매핑의 SSoT이고, CPU 측 짝은
`mod.rs::PrimitiveStyle::from_bar`다. **둘 중 하나만 바꾸면 색·두께·기준선이
조용히 어긋난다.**

| 필드 | 막대에서의 의미 |
|------|------------------|
| `color_premul` | 채움 색 (premultiplied) |
| `line_width_px` | 테두리 두께(픽셀) |
| `cap_half_px` | 이웃 막대 사이 간격(픽셀). 양쪽에 절반씩 |
| `cap_width_px` | bin 폭에서 막대가 차지하는 비율. `0..=1`로 clamp하며 가운데 정렬 |
| `shape_id` | 0 = edges가 x축을 따라간다(수직 막대), 1 = y축(수평 막대) |
| `dash[0]` | 테두리 색 (premultiplied) |
| `dash[1].xy` | 기준선(baseline)을 풀과 같은 `(hi, lo)` f32 쌍으로 |
| `point_radius_px` · `dash_len` · `primitive_flags` · `dash[1].zw` | 미사용 |

`dash`를 쓰는 이유: 막대는 dash 패턴이 없으므로 8개 f32가 비어 있고,
기준선을 `(hi, lo)` 쌍으로 담아야 큰 절대값에서도 정밀도가 유지된다
(풀의 논리값 표현과 동일). 단일 f32 필드에 담으면 그 정밀도가 사라진다.

`DataBarStyleConfig.bar_style_overrides`가 있으면 별도 mapped bar pipeline이
group 2의 sparse override table을 읽는다. override는 채움색, 테두리색·두께,
간격, 폭 비율만 바꾸며 baseline·orientation은 시리즈 단위로 유지한다. 렌더,
typed pick, 선택 outline은 모두 같은 override 해석 규칙을 사용한다.

### 2.2 `field_columnar.wgsl`의 `Style` 사용

면(heatmap/밴드)은 이 struct에서 **`color_premul` 하나만** 읽고, 그 의미는
`Config.colorbar.nan_color`(premultiplied)다 — 배치할 수 없는 z(NaN·로그에서
비양수·퇴화 범위)를 칠하는 색. `primitive_flags`를 포함한 나머지 필드는 읽지 않는다.

면의 나머지 파라미터는 `Style`을 재해석하지 않고 **자기 uniform**
(`FieldParams`, group 2 binding 4)에 담는다. 막대와 다른 선택인 이유: 면은
좌표 컬럼 lane base·격자 크기·방향·z 범위·레벨/스톱 개수까지 필요해서
`Style`의 빈 슬롯으로는 애초에 담기지 않는다. 그러면 재해석 표를 하나 더
만드는 것은 이득 없이 규칙만 늘리는 일이다.

group 2의 격자 storage + uniform은 면과 contour anchor가 공유하므로 이 SSoT의
**동기화 대상이다**. CPU 측 짝은 `mod.rs::FieldParamsGpu` / `GridColumnGpu`이고
그 문서 주석이 짝을 명시한다. 단, fragment lookup metadata인 binding 6은
`field_columnar.wgsl`만 선언하며 아래 field-only contour lookup metadata 계약을
따른다.

---

## 3. `maybe_log` — log10 인입/통과 헬퍼

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

매트릭스 격자에서 z를 읽고 셀 안에서 이중선형 보간·**해석적 기울기**까지 계산하는 블록.
채움(`field_columnar.wgsl`의 `fs_main`/`fs_contour`)과 라벨 앵커
(`contour_anchor.wgsl`의 Newton 투영)가 **같은 bind group·같은 함수**로 같은 답을 얻어야
하므로 SSoT다. 어긋나면 라벨이 자기 등고선에서 미묘하게 벗어난다.

`locate`가 `lattice` 인자를 받는 이유: 채움은 `Shading`이 정하는 **쿼드 격자**를 묻고, 등고선은
z가 실제로 사는 **샘플점 격자**를 묻는다. 두 질문이 한 이진 탐색을 공유한다.

`level_colors`(binding 5)는 앵커 셰이더가 읽지 않지만 블록에 들어 있다 — bind group 레이아웃이
한 벌이어야 하고, WGSL의 미사용 전역 선언은 합법이다.

CPU 짝: `mod.rs::GridColumnGpu` (8 B) · `mod.rs::FieldParamsGpu` (64 B) ·
`create_field_data_bind_group_layout`.

`gpu_errorbar.wgsl`의 field-fit 경로는 전체 sampling block을 복제하지 않고
아래 `*_grid_pair` 산술만 그대로 사용한다. `shader_consistency`가 각 함수 본문을
이 SSoT와 byte-for-byte 비교한다. 따라서 GPU가 돌려준 `(hi, lo)` 경계와 실제 field
shader가 축에 투영한 파생 경계가 서로 다른 반올림 규칙을 가질 수 없다.

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
    // `count` is bounded by the pool's column length, so 32 halvings settle it.
    for (var step = 0u; step < 32u; step = step + 1u) {
        if (hi - lo <= 1u) {
            break;
        }
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

### 5.1 field-only contour lookup metadata — group 2, binding 6

`field_columnar.wgsl`은 공통 블록 **밖에서만** binding 6을 선언한다. 각 8바이트
record는 `[finite_count, negative_infinity_count]`인 두 `u32`이고, 레벨 32개
블록 하나와 위치가 같다. `finite_count`는 binding 3 블록 앞쪽의 정렬된 유한 key
개수라 line/band 이진 탐색의 유일한 bound다. `negative_infinity_count`는 Bands의
분자에만 더하고 line 후보에는 넣지 않는다. NaN과 +Infinity는 둘 다 검색과 분자에서
제외한다. 레벨이 0개여도 WebGPU의 zero-sized storage binding을 피하려고
`[0, 0]` record 하나를 업로드한다.

CPU 짝은 `mod.rs::ContourLookupMetadataGpu` (8 B)와
`create_field_data_bind_group_layout`의 **fragment-only** binding 6이다. metadata
buffer는 `FieldTables`에서 나머지 group-2 table과 같이 만들어지고 bind group이
수명을 소유하며, 동일 `ChargeTally`의 `FieldTable` exact-byte lump charge에 포함된다.

---
## 변경 체크리스트

큰 변경 시 다음을 모두 확인:

- [ ] 본 문서의 해당 블록을 먼저 수정했다.
- [ ] `scatter_columnar.wgsl`의 common block을 수정했다.
- [ ] `line_columnar.wgsl`의 common block을 수정했다.
- [ ] `errorbar_columnar.wgsl`의 common block을 수정했다.
- [ ] `bar_columnar.wgsl`의 common block을 수정했다.
- [ ] `field_columnar.wgsl`의 common block을 수정했다.
- [ ] `line_arc.wgsl`의 common block을 수정했다 (Transform/maybe_log/
      data_to_ndc(vec2) 해당 시).
- [ ] `contour_anchor.wgsl`의 common block을 수정했다 (Transform 3블록 + 격자 샘플링 블록).
- [ ] `contour_label.wgsl`의 common block을 수정했다 (같은 3블록).
- [ ] `mod.rs::ScatterTransform` / `PrimitiveStyle`의 필드·바이트 크기를
      확인했다 (struct 크기가 바뀌었다면 `expected_size` 단정문도 갱신).
- [ ] `cargo check` 통과.
- [ ] `cargo test` 통과 (특히 pipeline compile tests).
