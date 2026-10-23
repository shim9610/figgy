# Shader common definitions (SSoT)

> **This file is NOT compiled.** WGSL has no `import`/`include`, so common
> struct/function definitions are duplicated into each shader file. The
> **single source of truth** for those duplicates lives here.
>
공통 정의를 수정할 때는 다음 순서를 따른다.

1. 이 문서의 해당 코드 블록을 먼저 수정한다.
2. 아래 동기화 대상 셰이더에서 같은 블록을 모두 수정한다.
3. 대응하는 CPU 구조체인 `mod.rs::ScatterTransform` / `PrimitiveStyle`의 크기와 필드 순서도 확인한다. `Transform`을 바꿨다면 공통 블록 밖의 사본인 `gpu_pick/core.wgsl`의 `PickQueryTransform`도 고친다(1절).
4. `cargo test -p figgy-renderer --test shader_consistency`로 공통 블록과 함수 쌍둥이를 검사한 뒤 `cargo check && cargo test`로 전체 빌드와 테스트를 실행한다.

한 파일만 바꾸면 컴파일은 성공해도 GPU가 메모리를 잘못 해석할 수 있다. 일부 시리즈의 색·위치·축만 어긋나는 오류를 피하려면 공통 정의를 함께 갱신해야 한다.

---

## 동기화 대상 셰이더

아래 셰이더는 이 문서의 공통 블록 가운데 자신에게 해당하는 블록만 복사해 둔다.

- `scatter_columnar.wgsl`, `line_columnar.wgsl`: `Transform`, `Style`, `maybe_log`, `axis_pair_to_t` / `data_to_ndc`, 좌표 표본 유효성의 다섯 블록.
- `errorbar_columnar.wgsl`, `bar_columnar.wgsl`: 좌표 표본 유효성을 뺀 네 블록.
- `field_columnar.wgsl`: 같은 네 블록과 격자 샘플링 블록(5절).
- `line_arc.wgsl`: 경로 길이 계산용. `Transform`, `maybe_log`, `axis_pair_to_t` / `data_to_ndc`, 좌표 표본 유효성 블록을 공유하고 `Style`은 사용하지 않는다. 공통 블록 밖의 계단 보간 헬퍼(`step_piece_count` / `step_piece` / `step_piece_len_px`)는 `line_columnar.wgsl`과 바이트 단위로 같아야 하며 `shader_consistency`가 검사한다.
- `contour_anchor.wgsl`: 라벨 앵커 계산용. 화면 좌표에서 위치를 선택하므로 `Transform`, `maybe_log`, `axis_pair_to_t` / `data_to_ndc` 세 블록과 격자 샘플링 블록(5절)을 사용한다. 좌표 표본 유효성 블록과 `Style`은 사용하지 않는다.
- `contour_label.wgsl`: 라벨 렌더링용. 앵커를 NDC에 배치하며 `Transform`, `maybe_log`, `axis_pair_to_t` / `data_to_ndc` 세 블록만 공유한다. 색과 배경은 아틀라스에 포함돼 있어 `Style`은 사용하지 않는다.
- `vector/project_cartesian2d.wgsl`: the Cartesian 2D projection fragment of
  the assembled vector shader (`vector.rs`). It carries the same five blocks
  as `scatter_columnar.wgsl` (`Transform`, `Style`, `maybe_log`,
  `axis_pair_to_t` / `data_to_ndc` and the sample validity rule); the other
  vector fragments contain no common block. See §5.3.

`field_columnar.wgsl::fs_contour`가 격자에서 등고선을 직접 그리고, `contour_anchor.wgsl`이 뉴턴 투영으로 앵커를 구한다. 따라서 두 셰이더가 격자 샘플링 블록을 공유한다.
라벨 투영은 `anchor_seed` → `anchor_step` 4회 → `anchor_project`의 최종 샘플·검증 →
`anchor_select` 순서로 실행한다. `anchor_step`은 뉴턴 보정 한 번이며 호스트가 같은
파이프라인을 네 번 dispatch한다. 격자 검색을 셰이더 안의 반복문으로 감싸면 소프트웨어 Vulkan
컴파일러가 최적화 과정에서 코드를 지나치게 펼치기 때문이다. 중간 `axis-t` 좌표는 후보
레코드의 `x`에 잠시 저장하고 최종 단계에서 데이터 좌표 쌍으로 덮어쓴다. 후보 버퍼는 선택
단계 전까지 외부에서 읽지 않으므로 중간 좌표를 위한 별도 버퍼나 바인딩이 없다.

`fullscreen_textured.wgsl`은 텍스처·샘플러용 별도 바인드 레이아웃을 사용하므로 이 문서의 공통 정의를 사용하지 않는다.

각 셰이더 안의 공통 블록은 다음 주석으로 감싸 두었다:

```wgsl
// ───── BEGIN common block (SHADER_COMMON.md) ─────
//        ...
// ───── END common block ─────
```

BEGIN과 END 주석 사이에는 이 문서에서 그 셰이더를 `applies-to`에 나열한 블록만, 문서에 나온 순서대로 빈 줄 하나를 사이에 두고 넣는다. 각 블록은 이 문서와 바이트 단위로 같아야 하며, 영역 안에 셰이더 고유의 코드나 주석이 있으면 `shader_consistency`가 실패한다. 대상 셰이더는 블록 바로 앞의 `<!-- shader-common: applies-to=... -->` 주석이 정한다. 이 주석은 렌더링된 문서에는 보이지 않으며, 주석과 코드 펜스 사이에는 빈 줄만 둘 수 있다. 주석의 짧은 이름(`scatter`, `arc`, `anchor`, `label`, `vector` 등)은 `tests/shader_consistency.rs`의 `shader_path_for`가 파일 경로로 바꾸므로, 새 셰이더를 대상에 추가하려면 같은 파일의 `TARGETS`와 `shader_path_for`도 고친다.

---

## 1. `Transform` 유니폼(group 0, binding 0)

`Transform`은 데이터 좌표를 NDC로 바꾸는 값, 로그축 여부, 픽셀과 NDC의 비율, 스케치·별자리 등 현재 스타일의 옵션을 전달한다. 크기는 144바이트다. `vec2<f32>` 8개, `array<vec4<f32>, 3>` 하나(바이트 오프셋 64, 원소 간 간격 16바이트), 그리고 극좌표용 `vec4<f32>` 2개(오프셋 112·128)로 구성된다.
점 반지름과 오차 막대 끝선 길이 같은 픽셀 크기는 `Style`에 저장한다. 셰이더는 `pixel_to_ndc`로 이를 NDC 크기로 환산한다.

The code is the fragment [`data_render/common/transform.wgsl`](common/transform.wgsl), spliced into each module that needs it by `wgsl_module!` (`shader_assembly.rs`).

| 필드 | 의미 |
|------|------|
| `data_min`, `data_max` | `Config.bottom_x`·`Config.left_y` 범위의 시작과 끝을 f32 쌍으로 나눈 상위(hi) 값. `.x`가 X축, `.y`가 Y축이다. 선형축은 설정값을, 로그축은 CPU에서 계산한 log10 값을 쓴다. 반전된 축은 두 값을 맞바꿔 담으므로 `data_min`에 최댓값이 들어간다. 데이터 영역 여백을 이유로 범위를 별도로 늘리지 않는다. |
| `data_min_lo`, `data_max_lo` | 같은 값의 하위(lo) 부분. `axis_pair_to_t`는 hi와 lo를 따로 빼서 큰 절댓값에서도 작은 차이를 유지한다. |
| `scale_log` | 축별 로그 변환 여부. 0.0은 선형, 1.0은 log10 |
| `pixel_to_ndc` | `(2/chart_w, 2/chart_h)`. `chart_w`·`chart_h`는 데이터 영역이 아닌 `Config.chart_area`의 픽셀 너비·높이다. 픽셀 크기를 NDC 크기로 바꾸는 비율이며 선 두께·점 반지름·오차 막대 끝선 길이에 사용한다. |
| `data_to_panel_scale`, `data_to_panel_offset` | 축 범위로 정규화한 좌표를 패널 좌표로 바꾸는 데이터 영역의 아핀 변환. 축 범위와 배치 여백을 분리한다. |
| `polar_angle`, `polar_extent` | `Config.polar`의 극좌표 상수. 직교 차트에서는 모두 0이다. `polar_angle` = (플래그, 0도 방향(라디안), 각도 단위당 라디안에 방향 부호(+1/−1)를 곱한 값, 안쪽 반지름 비율), `polar_extent` = (섹터 시작, 섹터 끝(각도 단위; 끝 ≤ 시작이면 전체 원), 세그먼트당 호 조각 수(현이면 0), 바깥 반지름 px). `line_columnar.wgsl`의 `line_step_mode`와 `gpu_pick/*.wgsl`의 선 판정은 호 조각 수를 이 값에서 읽어 `step_piece_count`의 모드 워드(비트 0–1 계단 모드, 비트 4 이상 호 조각 수)에 합친다. 플래그가 1이면 `data_to_ndc`가 `polar_ndc`로 분기해 각도 컬럼을 데이터 영역 중심 둘레의 각도로, 반지름 축의 정규화 좌표를 안쪽 고리와 바깥 반지름 사이로 사상한다. 극좌표는 `Scatter`·`Line`·`ScatterLine` 시리즈(면 채움 포함)에만 허용된다. 히스토그램·행렬·등고선·오차 막대·벡터 시리즈는 `renderer.rs::validate_polar_series`가 거부하므로 해당 셰이더에서는 이 분기가 실행되지 않는다. CPU 쌍둥이는 투영이 `polar_projection::ndc`, 표본 유효성이 `polar_projection::sample_valid`다. |
| `style_params` | Three generic style parameter slots. The active style's shader entries interpret them; the precise entries never read them. The slot layout is in the block comment above (sketch `[0]`, milkyway `[0]`–`[2]`, constellation `[0]`; every other lane is 0). Seeds are stored as f32 and recovered as `u32(...)`, exact up to 2^24. `[2].z` is reserved for the streamed point base under every style (see below). CPU packing: `StyleVariant::pack_params` in `renderer.rs` (`[f32; 12]`). |

**대응하는 CPU 구조체:** `src/data_render/mod.rs::ScatterTransform`
(`#[repr(C)]`, `bytemuck::Pod`). 필드 순서와 크기가 정확히 일치해야 한다.

`gpu_pick/core.wgsl`의 `PickQueryTransform`은 같은 배치를 공통 블록 밖에서 따로 선언한다. 픽 셰이더는 변환을 매개변수 유니폼 `PickQueryParams`의 첫 필드로 받고, CPU 쪽 `gpu_pick.rs::PickQueryParamsGpu`도 `ScatterTransform`을 그대로 담는다. `shader_consistency`는 이 사본을 검사하지 않고 `gpu_pick.rs`의 크기 단언(320바이트)은 크기 변화만 잡으므로, `Transform`을 바꿀 때 이 사본도 함께 고쳐야 한다.

`style_params[2].z` is reserved independently of the active style: it carries
the exact **u32 bit pattern** of a streamed point/errorbar chunk's global base,
not a numeric f32 conversion. `styled_point_index` adds the local instance id
as an integer, so identities above 2^24 keep all bits. Resident transforms write
zero. `create_stream_point_transform_bind_group` copies the whole 144-byte
transform, with the base written into `[2].z`, into a separately charged
immutable uniform; it never
rewrites the shared view transform or copies source payloads. Callers validate
the global range before creating the uniform. Line arc/star-slot identities
are separate and do not consume this field.

---

## 2. `Style` 유니폼(group 1, binding 0)

`Style`은 알파를 미리 곱한 색과 도형별 옵션을 전달한다. 크기는 80바이트, 정렬 단위는 16바이트다. 이 구조체를 쓰는 셰이더는 각자 필요한 필드만 읽는다.

The code is the fragment [`data_render/common/style.wgsl`](common/style.wgsl), spliced into each module that needs it by `wgsl_module!` (`shader_assembly.rs`).

| 필드 | 의미 |
|------|------|
| `color_premul` | premultiplied RGBA. `(r·a, g·a, b·a, a)` |
| `line_width_px` | 선 또는 오차 막대 몸통의 두께(px) |
| `point_radius_px` | 산점도 기호의 반지름(px) |
| `cap_half_px` | 오차 막대 끝선 길이의 절반(px) |
| `cap_width_px` | 오차 막대 끝선의 두께(px) |
| `shape_id` | `ScatterShape` GPU 코드 — 0 Circle, 1 Square, 2 Triangle, 3 Diamond, 4 Cross, 5 CircleFilled, 6 SquareFilled, 7 TriangleFilled, 8 DiamondFilled, 9 TriangleDown, 10 TriangleLeft, 11 TriangleRight, 12 Plus, 13 Pentagon, 14 Hexagon, 15 Octagon, 16 Star, 17 TriangleDownFilled, 18 TriangleLeftFilled, 19 TriangleRightFilled, 20 PlusFilled, 21 CrossFilled, 22 PentagonFilled, 23 HexagonFilled, 24 OctagonFilled, 25 StarFilled |
| `dash_len` | `dash`의 유효 스칼라 개수. 0은 실선 |
| `series_salt` | 시리즈마다 해시 패턴을 다르게 만드는 값. `renderer.rs`의 `create_style_for_series*`가 `fnv1a(series_id)`를 기록한다. 스케치·은하수·별자리 셰이더는 이 값을 시드에 XOR해 같은 X좌표를 쓰는 시리즈에서도 선의 흔들림과 별 패턴이 겹치지 않게 한다. 정밀 모드에서는 읽지 않는다. |
| `primitive_flags` | 도형별 기능 비트. 오차 막대는 bit 0으로 Y방향, bit 1로 X방향 사용 여부를 표시한다. 선 엔트리(`vs_main`/`vs_sketch`/`vs_ribbon`/`vs_area`)는 bit 2-3을 계단 보간 모드(0 직선, 1 StepPre, 2 StepPost, 3 StepMid)로 읽고, 면 채움 엔트리 `vs_area`는 bit 4를 "기준선 대신 다른 시리즈까지 채움"으로 읽는다. 다른 도형은 읽지 않는다. CPU 상수는 `line_step.rs`의 `LINE_STEP_SHIFT` / `AREA_FILL_TO_SERIES`이고, 계단 모드 비트는 `line_step::step_flag_bits`가 만든다. |
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

막대는 점선 패턴을 사용하지 않으므로 `dash`의 f32 슬롯 8개를 재사용한다. 기준선은 풀의 값과 같은 `(hi, lo)` 쌍으로 저장해 큰 값에서도 정밀도를 유지한다.

`DataBarStyleConfig.bar_style_overrides`를 지정하면 개별 막대 스타일용 파이프라인이 group 2의 덮어쓰기 표를 읽는다. 채움색·테두리색·두께·간격·너비 비율만 바꾸며 기준선과 방향은 시리즈 공통값을 유지한다. 그리기·데이터 피킹·선택 테두리는 같은 해석 규칙을 적용한다.

### 2.2 `line_columnar.wgsl`의 면 채움 엔트리 `vs_area` / `fs_area`

선 아래(또는 두 선 사이) 면 채움은 선 셰이더의 별도 엔트리와 별도 파이프라인("figgy line area pipeline")으로 그린다. 공통 `Style` 구조체를 그대로 쓰되 일부 필드를 막대처럼 다른 용도로 쓴다. CPU 쪽 작성자는 `renderer.rs::create_style_for_series_scaled_inner`(`area` 스타일)이며 아래 표와 일치해야 한다.

| 필드 | 면 채움에서의 의미 |
|------|------------------|
| `color_premul` | 알파를 미리 곱한 채움색 (`DataLineStyleConfig::effective_fill_color`) |
| `primitive_flags` | bit 2-3 계단 보간 모드(선과 동일), bit 4 = `LineFill::ToSeries`(슬롯 4/5의 두 번째 y 컬럼까지 채움). 0이면 `LineFill::ToBaseline` |
| `dash[1].xy` | 기준선을 풀과 같은 f32 쌍 `(hi, lo)`으로 저장 (막대와 같은 규칙). `ToSeries`에서는 읽지 않는다 |
| `series_salt` | 선과 같은 값 |
| 나머지 | 미사용 |

정점 슬롯 0-3은 선과 같고(x_a, y_a, x_b, y_b), 슬롯 4/5는 호 길이 대신 `(hi, lo)` 쌍으로 묶인 두 번째 y 컬럼(`y2_a`, `y2_b`)이다. 기준선 채움은 y 컬럼을 슬롯 4/5에 더미로 묶는다. 인스턴스 하나가 선분 하나이며 계단 모드에서는 선과 같은 조각 수만큼 사다리꼴을 낸다. 로그 y축에서 0 이하 기준선은 샘플처럼 가리지 않고 축 최솟값으로 고정한다(명시적 예외).

### 2.3 `field_columnar.wgsl`의 `Style` 사용

히트맵과 밴드 채움은 `color_premul`만 읽는다. 이 값은 알파를 미리 곱한 `Config.colorbar.nan_color`로, NaN·로그 스케일의 0 이하 값·너비가 없는 범위를 표시하는 색이다. `primitive_flags`를 포함한 다른 필드는 읽지 않는다.

나머지 행렬 옵션은 별도 유니폼인 `FieldParams`(group 2, binding 4)에 저장한다. 좌표 컬럼의 시작 위치, 격자 크기·방향, Z축 범위, 레벨·색상 기준점 개수는 `Style`의 빈 필드에 모두 담을 수 없기 때문이다.

group 2의 격자 스토리지와 유니폼은 행렬 렌더링과 등고선 앵커 계산이 공유하므로 동기화 대상이다. 대응하는 CPU 구조체는 `mod.rs::FieldParamsGpu` / `GridColumnGpu`다. 검색용 메타데이터인 binding 6은 `field_columnar.wgsl`에만 선언하며, 아래 5.1절의 규칙을 따른다.

---

<a id="3-maybe_log--log10-인입통과-헬퍼"></a>

## 3. 로그 변환 선택(`maybe_log`)

`is_log` 플래그(0.0 또는 1.0)에 따라 값을 그대로 통과시키거나 log10을
적용한다. 로그축은 `if`로 log10 값을 바로 돌려주고, 선형축은 원래대로 `mix`로
고른다. 선형축에서도 log10을 계산하지만 결과는 입력값 그대로다.

로그축을 `mix`로 고르면 안 된다. 많은 드라이버(Metal, DX12, 브라우저 WebGPU)가
`mix(x, y, a)`를 `x + a * (y - x)`로 계산하므로, `a = 1`이어도 `x`가 크면
`y - x`에서 `y`가 f32 반올림에 묻혀 `y`가 나오지 않는다. 로그축에서
`v ≥ 2²⁴`이면 log10(v)가 정수 단위로 깎이고 그보다 작은 값도 어긋난다.

선형축 계산은 바꾸지 않는다. `mix`를 `select`로 바꾸거나 첫 값에 `(1.0 - is_log)`를
곱했을 때, DX12 CI에서 큰 오프셋(1e9 이상)의 선형축 정밀도 테스트가 실패했다.
여러 셰이더가 같은 값을 비트 단위로 내야 하는 곳이라, 선형 경로는 원래 식을
그대로 둔다. 플래그로 두 값 중 하나를 고르는 곳(4절, 컬러맵 정규화,
`axis_px_per_unit_at`)은 모두 이 형태를 쓴다.

The code is the fragment [`data_render/common/projection.wgsl`](common/projection.wgsl), spliced into each module that needs it by `wgsl_module!` (`shader_assembly.rs`).

호출자는 `transform.scale_log.x` 또는 `transform.scale_log.y`를 `is_log`로
넣어 X·Y축을 독립적으로 선택한다.

---

## 4. hi/lo 데이터 좌표를 NDC로 변환(`axis_pair_to_t` / `data_to_ndc`)

Column pool logical values are `vec2<f32>(hi, lo)`. Linear axes compute the
range-local numerator as `(value_hi - min_hi) + (value_lo - min_lo)` so large
absolute timestamps still preserve small deltas after upload. Explicit `fma(1, a, b)`
preserves each pair addition against Metal fast-math reassociation without changing
the arithmetic result. Round-trip bitcasts are not rounding barriers. Log axes use the
recombined display value (`hi + lo`) before comparing against the log-transformed
axis bounds.

The code is the fragment [`data_render/common/projection.wgsl`](common/projection.wgsl), spliced into each module that needs it by `wgsl_module!` (`shader_assembly.rs`).

---

---

### Coordinate sample validity (line, scatter, arc and vector)

Nonpositive coordinates on logarithmic axes are omitted without changing source
columns. A line segment is omitted when either endpoint is invalid; arc length
also skips that segment so dashes and stars do not advance across the gap.
Vertices collapse to a finite position outside the viewport instead of depending
on NaN propagation. Bars and errorbar extents keep their own clipping rules.
In polar mode a sample whose radius is below the axis start is invalid too
(`polar_sample_valid`), so a line breaks there instead of folding through
the centre. Primitives that reach past the outer ring, into the hole or
across a sector edge (a sample outside the sector included) are cut there by
the fragment stage: every vertex entry of `scatter_columnar.wgsl` and
`line_columnar.wgsl` writes the `polar_px` / `polar_limits` / `polar_sector`
varyings (from `polar_clip_offset` / `polar_clip_limits` /
`polar_clip_sector`) and every fragment
entry starts with `polar_fragment_visible`, since the transform bind group
is vertex-visible only.

The code is the fragment [`data_render/common/sample_validity.wgsl`](common/sample_validity.wgsl), spliced into each module that needs it by `wgsl_module!` (`shader_assembly.rs`).

## 5. 격자 샘플링(group 2)과 공통 블록 밖의 정의

격자 샘플링 블록은 행렬의 Z값을 읽고 셀 내부의 이중선형 보간값과 해석적 기울기를 계산한다. 채움·등고선 렌더링(`field_columnar.wgsl`의 `fs_main` / `fs_contour`)과 라벨 앵커 계산(`contour_anchor.wgsl`의 뉴턴 투영)이 같은 바인드 그룹과 함수를 사용해야 한다. 서로 다르면 라벨이 등고선에서 벗어날 수 있다.

`locate`의 `lattice` 인자는 검색할 격자를 정한다. 채움은 `Shading`에 따른 사각형 격자를, 등고선은 실제 Z값이 있는 표본 격자를 사용한다. 두 경우 모두 같은 이진 탐색을 수행한다.

`level_colors`(binding 5)는 앵커 셰이더에서 읽지 않지만 바인드 그룹 배치를 맞추기 위해 공통 블록에 둔다. WGSL은 사용하지 않는 전역 선언을 허용한다.

대응하는 CPU 정의는 `mod.rs::GridColumnGpu`(8바이트), `mod.rs::FieldParamsGpu`(64바이트), `create_field_data_bind_group_layout`이다.

`gpu_errorbar.wgsl`의 행렬 범위 계산은 전체 샘플링 블록 대신 쌍 연산 함수 일곱 개(`grid_rounded_add`, `grid_rounded_subtract`, `normalize_grid_pair`, `add_grid_pairs`, `subtract_grid_pairs`, `scale_grid_pair`, `midpoint_grid_pair`)만 공유한다. `shader_consistency`가 함수 본문을 이 문서와 바이트 단위로 비교하므로 GPU 범위 계산과 실제 그리기가 같은 반올림 규칙을 사용한다.

The block is two fragments: [`data_render/common/grid_pair.wgsl`](common/grid_pair.wgsl) (the pair arithmetic) and [`data_render/common/field_grid.wgsl`](common/field_grid.wgsl) (`GridColumn`, the `FIELD_*` flags, `FieldParams`, the group 2 bindings and the sampling), spliced in that order by `wgsl_module!` (`shader_assembly.rs`).

<a id="51-field-only-contour-lookup-metadata--group-2-binding-6"></a>

### 5.1 행렬 셰이더의 등고선 검색 메타데이터(group 2, binding 6)

`field_columnar.wgsl`은 공통 블록 밖에서 binding 6을 선언한다. 레벨 32개짜리 블록마다 `[finite_count, negative_infinity_count]`라는 u32 두 개, 총 8바이트를 기록한다.
`finite_count`는 binding 3 블록 앞부분에 정렬해 둔 유한 레벨 수이며 선·밴드 이진 탐색의 범위를 정한다. `negative_infinity_count`는 밴드 계산의 분자에만 더하고 선 후보에는 포함하지 않는다. NaN과 양의 무한대는 검색과 분자에서 모두 제외한다. 레벨이 없어도 WebGPU의 크기 0인 스토리지 바인딩을 피하려고 `[0, 0]` 하나를 업로드한다.

CPU 구조체는 `mod.rs::ContourLookupMetadataGpu`(8바이트)이며 `create_field_data_bind_group_layout`의 프래그먼트 전용 binding 6에 연결된다. 버퍼는 `create_field_data_bind_group`이 `FieldTables`로 받은 다른 group 2 표와 함께 만들고, 반환하는 바인드 그룹이 보관한다. 할당량은 다른 표와 같은 `ChargeTally`에 실제 바이트 수로 합산해 `GpuResourceKind::FieldTable` 항목으로 기록한다.

### 5.2 Point value maps — group 3 (mapped scatter entries)

Outside the common block, `scatter_columnar.wgsl`'s mapped entries (`vs_mapped`,
`vs_pick_ring_mapped`) bind group 3 for a series' continuous value maps
(`point_color_map` / `point_size_map`). Binding 0 is `PointValueMapParams`
(64 B; CPU twin `mod.rs::PointValueMapParamsGpu`): the colourbar's bounds as
hi/lo pairs, already log10 when `VALUE_MAP_LOG_Z` is set, the size domain as
hi/lo pairs, `(min_px, max_px)` already multiplied by the export scale, the
flags, the stop count and the premultiplied `nan_color`. Binding 1 is the
colourbar's ramp control points, straight RGBA, at least one entry. A mapped
series without value maps binds the renderer's neutral params (every flag
clear), so the layout is always satisfied.

The value columns arrive as per-instance vertex slots 4 and 5 (hi/lo pairs,
X bound as filler when a map is absent). `value_ramp_t` and `value_map_radius`
are one-line wrappers over the shared ramp and size-map functions of §5.4 (the
field and vector shaders carry the same ramp, the pick shader the size map),
and `value_ramp` adds only the stop lookup, with its zero- and one-stop cases,
around `ramp_segment` / `ramp_blend`. Precedence inside
`resolve_mapped_style` is override > value map > table slot > base; a hidden
point (non-finite size value) carries a negative radius and is masked unless an
override sets its radius. Nothing derived per point is uploaded: colour and
radius are computed on every draw from the bound columns.

### 5.3 Vector fields — group 2 (assembled vector shader)

The vector shader is not one file: `vector.rs::assemble` concatenates
`vector/project_cartesian2d.wgsl` (the five common blocks above plus the
projection), `vector/vector_input_common.wgsl`, one of
`vector/vector_input_points.wgsl` / `vector/vector_input_grid.wgsl`,
`common/arrow.wgsl` and `vector/vector_entry.wgsl`, in that order. Only
the projection fragment carries common blocks; the consistency test compares
it as the `vector` target. `vector/vector_input_grid.wgsl` also carries
renamed copies of the §5 pair functions (`vector_rounded_add`,
`vector_rounded_subtract`, `vector_normalize_pair`, `vector_add_pairs`,
`vector_scale_pair` and `vector_midpoint_pair`, twins of `grid_rounded_add`,
`grid_rounded_subtract`, `normalize_grid_pair`, `add_grid_pairs`,
`scale_grid_pair` and `midpoint_grid_pair`) so that an `Edges` arrow sits on
the field's cell centre. No test compares these copies, so a change to those
§5 functions has to be copied into them by hand.

Group 2 is the series' data. Binding 0 is the column pool as read-only
storage (hi/lo pairs, like the field shader's binding 0). Binding 1 is the
grid table (`GridColumn { base, len }`, laid out like the field shader's
binding 1; CPU twin `mod.rs::GridColumnGpu`): one padding entry for the
points form, and for the grid form the `u`, `v` and colour grids' columns
appended in that order, each addressed from its own base in `VectorParams`. Binding 2 is the colourbar's ramp control points (straight
RGBA, at least one entry). Binding 3 is `VectorParams` (128 B; CPU twin
`vector.rs::VectorParamsGpu`): the x / y / u / v / colour lanes and lengths,
the grid's `cols` / `rows`, the flags, the arrow count, the stop count, the
pivot, the scale (`Data`: the factor; `Pixels`: `px_per_unit` times the
export scale), the pixel sizes (shaft width, head length, head width, minimum
length and the anti-aliasing width, every one already multiplied by the
export scale) and the colourbar's bounds as hi/lo pairs, already log10 under
`VECTOR_FLAG_LOG_Z`, with the premultiplied `nan_color`.

Flags: `GRID` selects the grid input, `COLUMNS_ARE_Y` and `CENTERS` are the
`u` grid's orientation and layout, `SCALE_PIXELS` the scale mode, `LOG_Z` the
colourbar's scale and `COLOR_MAP` whether a colour column or grid is bound.
`vector_sample(i)` returns the anchor as hi/lo pairs, the (u, v) components
and the colour value: under `Centers` the anchor is the coordinate, under
`Edges` the data-space midpoint of two consecutive coordinates (the field's
cell centre). `project_arrow` turns a sample into a tail and a tip in
chart-area pixels: `Data` adds `scale * (u, v)` to the anchor through
`pair_add` and projects both ends, so the arrow follows the axis transform;
`Pixels` takes the direction from the axes' pixel-per-unit Jacobian
(`axis_px_per_unit`) and the length from `scale * |(u, v)|`. `vector_ramp_t`
and `vector_ramp` wrap the shared ramp functions of §5.4, so a colour-mapped
arrow takes the colour a heatmap cell of the same value would.

The glyph fragment owns nothing but pixel geometry: `arrow_place` applies the
pivot and the minimum length and scales the head down to a short arrow,
`arrow_quad_corner` places the six vertices of the covering quad, and
`arrow_coverage` evaluates the signed distance to shaft ∪ head
(`arrow_distance`) per fragment. The quad reaches half the shaft width plus
the margin past both ends, so the shaft's round end is never cut. Nothing
derived per arrow is uploaded: every draw recomputes the placement from the
bound columns.

Picking follows the draw. A grid vector's typed data pick is the grid input
fragment's compute entry `pick_vector_cell` (bound like the draw, group 0 the
pick transform, group 1 the typed pick query and output, group 2 this
series' group): it walks every instance through `vector_sample`,
`project_arrow`, `arrow_place` and `arrow_distance` and answers the matrix
cell of the nearest drawn arrow, a later instance winning an exact tie. A
point vector is a point pick (§5.8).

### 5.4 Colour ramp and size-map radius — function twins

The colourbar ramp is evaluated in three shaders (`field_columnar.wgsl` for
cells, `scatter_columnar.wgsl` for `point_color_map`, the vector shader for
`VectorStyle::color_map`) and the size-map radius in two
(`scatter_columnar.wgsl` and `gpu_pick/*.wgsl`). The maths is one function set,
parameterised on the caller's bounds, so the stop storage binding (which WGSL
cannot pass as an argument) stays the only per-shader line. The functions
below are copied byte-for-byte into each shader and `shader_consistency`
compares every body with this document (`ramp_t` / `ramp_segment` /
`ramp_blend` in field, scatter and vector; `size_map_radius` in scatter and
pick; `ramp_finite` everywhere). CPU twins: `ColorBarOptions::normalized_z`,
`model::colormap::sample` and `PointSizeMap::radius_px`.

The colour ramp is the fragment [`data_render/common/ramp.wgsl`](common/ramp.wgsl) and the size-map radius is part of [`data_render/common/marker.wgsl`](common/marker.wgsl), spliced into each module that needs them by `wgsl_module!` (`shader_assembly.rs`). The pick modules carry their own `ramp_finite`: the ramp fragment calls `maybe_log`, which they do not have.

### 5.5 Marker shape — function twins

A marker is drawn by the scatter shader (`fs_main`, `fs_mapped`,
`fs_sketch`) and picked by `gpu_pick/*.wgsl`, and both must answer the same
question: does this pixel belong to the marker? The shape is one function
set: `shape_distance` and its helpers give the signed pixel distance from the
marker's local position `p` (y up, like NDC) to the contour of the base
shape, and `marker_edge` turns it into the coverage edge — a filled marker
covers `d < 0`, an outline (and an open plus or cross) draws a stroke of
half-width `STROKE_HALF_PX` centred on `d == 0`. The fragment covers its pixel
where the edge is below the anti-aliasing width; the picker's hit distance is
`max(marker_edge(d, filled), 0)` evaluated at the cursor, with the canvas y
axis flipped into the marker's frame. So a pick at distance zero is a pixel
at least half covered, and every painted pixel is within one anti-aliasing
width of a zero-distance pick. The radius is the drawn one: the series or
mapped radius times the display scale, through `visual_shape_radius`; the
stroke half-width is in physical pixels and does not scale.

Nothing a marker draws lies farther than `visual_shape_radius +
QUAD_MARGIN_PX` from its centre on either axis (the quad `vs_main` emits),
which is the reach the picker's axis gates allow around each centre.

The functions and constants below are copied byte-for-byte into
`scatter_columnar.wgsl` and `gpu_pick/*.wgsl`; `shader_consistency` compares
every body and constant with this document. CPU twin: the `marker_*`
functions of `pick.rs` (`styled_marker_distance`).

The functions and constants are the fragment [`data_render/common/marker.wgsl`](common/marker.wgsl), spliced into the scatter and pick marker modules by `wgsl_module!` (`shader_assembly.rs`).

The picked ring (`vs_pick_ring`, `vs_pick_ring_mapped`) is drawn around the
same marker: its centreline lies `radius_extra_px` (`Style.cap_half_px`)
past `marker_extent_px`, the farthest point where `marker_edge <= 0` (the
contour's circumradius at the visual radius, plus `STROKE_HALF_PX` for an
outline), taken at the drawn radius and shape (the mapped entry resolves
both per point; a size map's hidden point draws no ring). Under a stylized
draw style `ring_marker_extent_px` uses the §5.7 envelope the picker picks:
the sketch wobble added, a Milkyway planet's ring band or rim glow, a
Constellation star's sprite disc. These are scatter-shader functions only;
`pick_draw_agreement` measures the ring against the exported marker.

### 5.6 Line stroke — function twins

A line is drawn by `line_columnar.wgsl` (`vs_main` / `vs_sketch` with
`fs_main` or `fs_constellation_line`) and picked by `gpu_pick/*.wgsl`, and both
answer the same question per pixel. Every step piece of a segment is
extruded into a rectangle with square caps, `max(line_width_px, 1) / 2` wide
either side: `line_stroke_distance` is its signed distance in the piece's
`(along, side)` frame, and the stroke covers a pixel where it is below the
anti-aliasing width. A dashed stroke is on where `dash_pattern_on` says so at
the interpolated arc position, which is the cumulative arc prefix of
`line_arc.wgsl` at the piece's ends (`arc_start - half_w` at the A-side
corners, `arc_end + half_w` at the B-side ones). A sketch stroke subdivides
each piece into `SKETCH_SUBDIV` spans whose knots are displaced along the
normal by `sketch_line_offset` at their arc position, and the fragment
evaluates the same rectangle in the undisplaced `(along, side)` frame.

The picker inverts that extrusion at the cursor: it projects the cursor onto
the piece, removes the knot displacement interpolated at that position, and
evaluates `line_stroke_distance` and `dash_pattern_on` there. Its exact pass
binds the line's arc prefix (group 2, `pick_arc`) — the buffer the line was
drawn with, from the renderer's arc cache under the same key — so the dash
phase and the wobble are the drawn ones. A streamed series is picked without
it (streaming is not extended): its dashes pick as a solid stroke and its
sketch wobble as the envelope `±amplitude_px`. In a dash gap the hit distance
runs along the stroke to the nearer dash of the same piece. A Milkyway
ribbon is never dashed; its strip and the star field scattered along it pick
as one rectangle of the larger half extent (`gpu_pick.rs::
milkyway_line_half_extent`), since the star positions are hashed per slot.

The functions and constants below are copied byte-for-byte into
`line_columnar.wgsl` and `gpu_pick/*.wgsl` (the sketch noise also into
`scatter_columnar.wgsl` and `errorbar_columnar.wgsl`); `shader_consistency`
compares every body and constant with this document. CPU twin:
`pick.rs::line_segment_pick` (precise strokes and dashes) with its arc sum.

The stroke constants, `line_stroke_distance`, `dash_pattern_on` and `sketch_line_offset` are the fragment [`data_render/common/line_stroke.wgsl`](common/line_stroke.wgsl), spliced into the line and pick modules by `wgsl_module!` (`shader_assembly.rs`). The noise they use is [`data_render/common/sketch_noise.wgsl`](common/sketch_noise.wgsl).

### 5.7 Styled markers — function twins

The stylized draw styles replace the precise marker with their own entry, and
the picker follows the entry the chart's style draws with
(`gpu_pick.rs::GpuPickDrawStyle`, the shader's `styled` modes):

- **Sketch** (`fs_sketch`) perturbs `shape_distance` by
  `sketch_marker_distance`'s angular noise, seeded per marker from the
  style seed, the series salt (`Style.series_salt`, `fnv1a(series_id)`) and
  the global point index; the picker evaluates the same function, so a
  sketch marker picks exactly. Sketch strokes are §5.6's.
- **Milkyway** draws a planet (`fs_planet`): the disc, the rim glow where
  `planet_rim_glow` stays above `PLANET_ALPHA_FLOOR`, and the ring band
  between `RING_INNER` and `RING_OUTER` at the hashed position angle
  `planet_ring_phi`, all inside the quad `vs_planet` emits. The picker uses
  the same functions; the ring band is the envelope of the baked ring
  texture, whose density is data, not a shader constant.
- **Constellation** draws a PSF star (`fs_constellation_star`) of radius
  `constellation_star_radius` in a quad of four times that plus
  `QUAD_MARGIN_PX`. Its brightness and tint are hashed and its sprite is a
  baked texture that fades to zero inside the disc the quad inscribes
  (`mod.rs::bake_psf_rgba`), so that disc is what picks. A zero star or line
  opacity draws nothing and picks nothing.

The functions and constants below are copied byte-for-byte into
`scatter_columnar.wgsl` and `gpu_pick/*.wgsl`; `shader_consistency` compares
every body and constant with this document. CPU twin:
`pick.rs::styled_marker_distance`.

The functions and constants are the end of the fragment [`data_render/common/marker.wgsl`](common/marker.wgsl), after the marker shape and the size-map radius.

### 5.8 Arrow glyph — function twins

A point vector series (`DataRenderType::Vector` with `VectorComponents::Points`)
is picked on the arrow the assembled vector shader draws, not at its anchor.
The pick shader's `pick_vector_candidate` repeats the draw's `vector_sample`
(the shortest of the anchor, component and colour columns bounds the arrow
count; a non-finite component draws nothing) and `project_arrow` (the `Data`
tip through the axes, or the `Pixels` direction through
`axis_px_per_unit_at`), then calls the glyph's own `arrow_place` and
`arrow_distance`. The draw frame is y-up framebuffer pixels; the picker works
in canvas pixels flipped up (`pick_flip`), a translation of it, so distances
agree. A zero vector, an arrow under `min_length_px` and an arrow with a
non-finite end are not drawn and do not pick.

`ARROW_SHAFT_MIN_PX` keeps the head-only arrow (shorter than its head) from
growing a round dot at the tail: the fragment stage recomputes the length from
the pivot-shifted tail and tip, and that rounding used to leave a shaft of a
few ulps whose round end the draw painted and the pick did not.

The items below are copied byte-for-byte into `common/arrow.wgsl` (the
constants, structs and `arrow_*` functions), `vector/project_cartesian2d.wgsl`
(`pair_add`, `axis_px_per_unit_at`) and `gpu_pick/*.wgsl`; `shader_consistency`
compares every body, struct and constant with this document. CPU twin:
`pick.rs::vector_arrow_distance` with `arrow_place` / `arrow_distance`.

The glyph (the `ARROW_*` constants, `ArrowStyle`, `ArrowPlacement` and the `arrow_*` functions), `pair_add` and `axis_px_per_unit_at` are the fragment [`data_render/common/arrow.wgsl`](common/arrow.wgsl), spliced into the vector modules by `wgsl_module!` (`shader_assembly.rs`); the pick vector fragment still carries a copy of its picking subset.

---
## 변경 체크리스트

공통 정의를 변경했다면 다음 항목을 모두 확인한다.

- [ ] 본 문서의 해당 블록을 먼저 수정했다.
- [ ] `scatter_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `line_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `errorbar_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `bar_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `field_columnar.wgsl`의 공통 블록을 수정했다.
- [ ] `line_arc.wgsl`의 공통 블록을 수정했다 (`Transform`, `maybe_log`,
      `axis_pair_to_t` / `data_to_ndc`, 좌표 표본 유효성 블록 해당 시).
- [ ] `contour_anchor.wgsl`의 공통 블록을 수정했다 (Transform 3블록 + 격자 샘플링 블록).
- [ ] `contour_label.wgsl`의 공통 블록을 수정했다 (같은 3블록).
- [ ] `vector/project_cartesian2d.wgsl`의 공통 블록을 수정했다 (scatter와 같은 5블록).
- [ ] `mod.rs::ScatterTransform` / `PrimitiveStyle`의 필드와 바이트 크기를
      확인했다 (struct 크기가 바뀌었다면 `mod.rs`의
      `size_of::<ScatterTransform>() == 144` / `size_of::<PrimitiveStyle>() == 80`
      컴파일 타임 단언도 갱신).
- [ ] `Transform`을 바꿨다면 `gpu_pick/core.wgsl`의 `PickQueryTransform`도 같은
      배치로 고쳤다 (크기가 바뀌었다면 `gpu_pick.rs`의
      `size_of::<PickQueryParamsGpu>() == 320` 단언도 갱신).
- [ ] 격자 샘플링 블록을 바꿨다면 `mod.rs::FieldParamsGpu` / `GridColumnGpu`를
      확인했다. 쌍 연산 함수를 바꿨다면 `gpu_errorbar.wgsl`의 같은 함수와
      `vector/vector_input_grid.wgsl`의 `vector_*` 사본도 고쳤다.
- [ ] 5.4절 함수를 바꿨다면 `field_columnar.wgsl`, `scatter_columnar.wgsl`,
      `vector/vector_input_common.wgsl`, `gpu_pick/*.wgsl`의 사본을 고쳤다.
- [ ] 5.5절 마커 모양 함수를 바꿨다면 `scatter_columnar.wgsl`과
      `gpu_pick/*.wgsl`의 사본, `pick.rs`의 `marker_*` CPU 쌍둥이를 고쳤다.
- [ ] 5.6절 선 획 함수를 바꿨다면 `line_columnar.wgsl`과 `gpu_pick/*.wgsl`의
      사본, `pick.rs::line_segment_pick` CPU 쌍둥이를 고쳤다.
- [ ] 5.7절 양식 마커 함수를 바꿨다면 `scatter_columnar.wgsl`과
      `gpu_pick/*.wgsl`의 사본, `pick.rs::styled_marker_distance` CPU 쌍둥이를
      고쳤다.
- [ ] 5.8절 화살표 글리프 함수를 바꿨다면 `common/arrow.wgsl`,
      `vector/project_cartesian2d.wgsl`, `gpu_pick/*.wgsl`의 사본,
      `pick.rs::vector_arrow_distance` CPU 쌍둥이를 고쳤다.
- [ ] 계단 보간 헬퍼를 바꿨다면 `line_columnar.wgsl`과 `line_arc.wgsl`을 함께
      고쳤고, `gpu_pick/*.wgsl`의 `pick_step_piece_count` / `pick_step_piece`가
      같은 규칙을 따르는지 확인했다.
- [ ] `cargo test -p figgy-renderer --test shader_consistency` 통과.
- [ ] `cargo check` 통과.
- [ ] `cargo test -p figgy-renderer` 통과 (셰이더 컴파일 테스트
      `src/shader_compile_tests.rs`는 GPU 어댑터가 필요하다).
