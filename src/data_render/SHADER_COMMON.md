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

데이터 좌표 → NDC 변환, 로그 축 플래그, 점/캡 크기, 픽셀↔NDC 환산 비율을
셰이더에 전달하는 유니폼. **48바이트, 16바이트 정렬** (WGSL std140-like).

<!-- shader-common: applies-to=scatter,line,errorbar -->
```wgsl
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    point_size_ndc: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> transform: Transform;
```

| 필드 | 의미 |
|------|------|
| `data_min`, `data_max` | NDC로 매핑할 데이터 좌표 범위(X, Y) |
| `point_size_ndc` | scatter 점 반지름 / errorbar cap 반-너비 — NDC 단위, per-axis |
| `scale_log` | per-axis 로그 플래그. 0.0 = linear, 1.0 = log10 |
| `pixel_to_ndc` | `(2/chart_w, 2/chart_h)` — 1픽셀이 NDC에서 몇인지. line 두께 환산에 쓰임 |
| `_pad` | 16바이트 정렬 유지용 |

**CPU 측 짝:** `src/data_render/mod.rs::ScatterTransform`
(`#[repr(C)]`, `bytemuck::Pod`). 필드 순서·크기 1:1 일치해야 한다.

---

## 2. `Style` uniform — group 1, binding 0

색(premultiplied alpha)과 per-primitive 옵션. **32바이트, 16바이트 정렬.**

<!-- shader-common: applies-to=scatter,line,errorbar -->
```wgsl
struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
};

@group(1) @binding(0) var<uniform> style: Style;
```

| 필드 | 의미 |
|------|------|
| `color_premul` | premultiplied RGBA. `(r·a, g·a, b·a, a)` |
| `line_width_px` | line 두께(픽셀). 다른 셰이더는 무시 |

**CPU 측 짝:** `src/data_render/mod.rs::PrimitiveStyle`
(`#[repr(C)]`, `bytemuck::Pod`). 패딩 포함 32바이트.

---

## 3. `maybe_log` — log10 인입/통과 헬퍼

`is_log` 플래그(0.0 또는 1.0)에 따라 값을 그대로 통과시키거나 log10을
적용한다. `if` 분기 없이 `mix`로 처리해 워프 단위 분기 비용을 피한다.

<!-- shader-common: applies-to=scatter,line,errorbar -->
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

### 4a. `vec2<f32>` 인자 형태 (errorbar)

<!-- shader-common: applies-to=errorbar -->
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
- [ ] `mod.rs::ScatterTransform` / `PrimitiveStyle`의 필드·바이트 크기를
      확인했다 (struct 크기가 바뀌었다면 `expected_size` 단정문도 갱신).
- [ ] `cargo check` 통과.
- [ ] `cargo test` 통과 (특히 pipeline compile tests).
