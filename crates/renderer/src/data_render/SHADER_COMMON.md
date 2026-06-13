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
- `line_arc.wgsl` (컴퓨트 — Transform / `maybe_log` / `data_to_ndc`(vec2)만
  공유, `Style` 은 사용하지 않음)

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
유니폼. **80바이트** (`vec2<f32>` 4개 + `array<vec4<f32>, 3>` 1개 —
배열은 offset 32, 원소 stride 16, WGSL uniform layout). 픽셀 단위
크기(점 반지름, cap 길이 등)는 `Style`로 이동했다 — 픽셀→NDC 환산은
셰이더가 `pixel_to_ndc`로 직접 수행한다.

<!-- shader-common: applies-to=scatter,line,errorbar,arc -->
```wgsl
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
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
    style_params: array<vec4<f32>, 3>,
};  // 80 B (vec4 array at offset 32, stride 16 — alignment unchanged)

@group(0) @binding(0) var<uniform> transform: Transform;
```

| 필드 | 의미 |
|------|------|
| `data_min`, `data_max` | NDC로 매핑할 데이터 좌표 범위(X, Y) |
| `scale_log` | per-axis 로그 플래그. 0.0 = linear, 1.0 = log10 |
| `pixel_to_ndc` | `(2/chart_w, 2/chart_h)` — 1픽셀이 NDC에서 몇인지. 픽셀 단위 크기(line 두께, 점 반지름, cap 길이) 환산에 쓰임 |
| `style_params` | Generic style parameter slots. Interpretation belongs to the active styled entry. sketch: `[0]`=(amplitude_px, wavelength_px, seed(f32), 0), rest 0. milkyway: `[0]`=(star_density, ribbon_width_px, ribbon_intensity, seed(f32)), `[1]`=(star_scale, spread_px, faint_bias, planet_rim), `[2]`=(structure_scale, star_brightness, 0, 0). constellation: `[0]`=(star_opacity, line_opacity, 0, 0), rest 0. Seeds are stored as f32 and recovered as `u32(...)`; exact up to 2^24. CPU packing lives in renderer.rs (`StyleVariant::pack_params`, `[f32; 12]`). Precise entries do not read these slots. |

**CPU 측 짝:** `src/data_render/mod.rs::ScatterTransform`
(`#[repr(C)]`, `bytemuck::Pod`). 필드 순서·크기 1:1 일치해야 한다.

---

## 2. `Style` uniform — group 1, binding 0

색(premultiplied alpha)과 per-primitive 옵션. **80바이트, 16바이트 정렬.**
세 셰이더가 같은 struct를 공유하고 각자 자기 필드만 읽는다(나머지는 무시).

<!-- shader-common: applies-to=scatter,line,errorbar -->
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
    _pad: u32,
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
| `shape_id` | `ScatterShape` 선언 순서 인덱스 0..8 — 0 Circle, 1 Square, 2 Triangle, 3 Diamond, 4 Cross, 5 CircleFilled, 6 SquareFilled, 7 TriangleFilled, 8 DiamondFilled |
| `dash_len` | `dash`의 유효 스칼라 개수. 0 = solid |
| `series_salt` | 시리즈 간 해시 탈상관 솔트 — `fnv1a(series_id)` (renderer.rs `create_style_for_series*`가 기록). 스케치/성좌 entry가 자기 해시 시드에 XOR한다. 같은 x 격자를 쓰는 시리즈들이 wobble/별 패턴을 공유하지 않게 하는 장치. 정밀 entry는 읽지 않음 |
| `_pad` | `dash`의 16바이트 정렬 유지용 |
| `dash` | 최대 8개의 순차 `[on, off, ...]` 픽셀 길이 — `dash[0].xyzw` 먼저, 이어서 `dash[1].xyzw` |

**CPU 측 짝:** `src/data_render/mod.rs::PrimitiveStyle`
(`#[repr(C)]`, `bytemuck::Pod`). 패딩 포함 80바이트. `shape_id` 매핑은
`mod.rs::shape_id()` 헬퍼가 담당한다.

---

## 3. `maybe_log` — log10 인입/통과 헬퍼

`is_log` 플래그(0.0 또는 1.0)에 따라 값을 그대로 통과시키거나 log10을
적용한다. `if` 분기 없이 `mix`로 처리해 워프 단위 분기 비용을 피한다.

<!-- shader-common: applies-to=scatter,line,errorbar,arc -->
```wgsl
fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}
```

호출자는 `transform.scale_log.x` 또는 `transform.scale_log.y`를 `is_log`로
넣어 X·Y축을 독립적으로 선택한다.

---

## 4. `data_to_ndc` — 데이터 좌표 → NDC

X·Y에 각각 `maybe_log`을 적용한 뒤 `[data_min, data_max] → [-1, 1]`로
선형 매핑.

`line_columnar.wgsl`은 `(x: f32, y: f32)` 두 인자 형태, `errorbar_columnar.wgsl`은
`vec2<f32>` 한 인자 형태로 약간 다른 시그니처를 쓴다(둘 다 SSoT). 의미는
동일하니, 어느 한쪽을 바꾸면 다른 쪽도 같은 의미로 바꾼다.

### 4a. `vec2<f32>` 인자 형태 (errorbar, arc)

<!-- shader-common: applies-to=errorbar,arc -->
```wgsl
fn data_to_ndc(v: vec2<f32>) -> vec2<f32> {
    let xv = maybe_log(v.x, transform.scale_log.x);
    let yv = maybe_log(v.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    return t * 2.0 - 1.0;
}
```

### 4b. `(f32, f32)` 인자 형태 (line)

<!-- shader-common: applies-to=line -->
```wgsl
fn data_to_ndc(xv: f32, yv: f32) -> vec2<f32> {
    let xv2 = maybe_log(xv, transform.scale_log.x);
    let yv2 = maybe_log(yv, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv2, yv2) - transform.data_min) / range;
    return t * 2.0 - 1.0;
}
```

`scatter_columnar.wgsl`은 `vs_main` 본체에 같은 식을 인라인해 두었다
(별도 함수로 분리하지 않음). 수정 시 인라인 식도 같은 의미로 갱신할 것.

---

## 변경 체크리스트

큰 변경 시 다음을 모두 확인:

- [ ] 본 문서의 해당 블록을 먼저 수정했다.
- [ ] `scatter_columnar.wgsl`의 common block을 수정했다.
- [ ] `line_columnar.wgsl`의 common block을 수정했다.
- [ ] `errorbar_columnar.wgsl`의 common block을 수정했다.
- [ ] `line_arc.wgsl`의 common block을 수정했다 (Transform/maybe_log/
      data_to_ndc(vec2) 해당 시).
- [ ] `mod.rs::ScatterTransform` / `PrimitiveStyle`의 필드·바이트 크기를
      확인했다 (struct 크기가 바뀌었다면 `expected_size` 단정문도 갱신).
- [ ] `cargo check` 통과.
- [ ] `cargo test` 통과 (특히 pipeline compile tests).
