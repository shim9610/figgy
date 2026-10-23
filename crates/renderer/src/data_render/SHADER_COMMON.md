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
    return fma(1.0, a, b);
}

fn grid_rounded_subtract(a: f32, b: f32) -> f32 {
    return fma(1.0, a, -b);
}

/// Normalize a finite pair with error-free addition. Explicit fma additions
/// preserve rounding boundaries on Metal; round-trip bitcasts are eliminated
/// by optimizers and do not prevent reassociation.
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
    let span = grid_rounded_subtract(b, a);
    if (!f32_is_finite(a) || !f32_is_finite(b) || !f32_is_finite(span)) {
        return out;
    }
    out.index = lo;
    // A zero-width quad (a lone centre) has no inside; treat it as its start.
    var frac = 0.0;
    if (span != 0.0) {
        frac = grid_rounded_subtract(t, a) / span;
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
`vector/glyph_arrow.wgsl` and `vector/vector_entry.wgsl`, in that order. Only
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

The colour ramp is the fragment [`data_render/common/ramp.wgsl`](common/ramp.wgsl), spliced into each module that needs it by `wgsl_module!` (`shader_assembly.rs`). The size-map radius is still copied:

```wgsl
/// The marker radius for a size-map value `sv` (a hi/lo pair) over the
/// domain `domain_min..domain_max` (hi/lo pairs) onto `lo..hi` pixels, by
/// marker area when `area`, else by radius. Negative when the value is not
/// finite: the point is hidden. Twin of `PointSizeMap::radius_px`.
fn size_map_radius(sv: vec2<f32>, domain_min: vec2<f32>, domain_max: vec2<f32>, lo: f32, hi: f32, area: bool) -> f32 {
    if (!ramp_finite(sv.x) || !ramp_finite(sv.y) || !ramp_finite(sv.x + sv.y)) {
        return -1.0;
    }
    let num = (sv.x - domain_min.x) + (sv.y - domain_min.y);
    let span = (domain_max.x - domain_min.x) + (domain_max.y - domain_min.y);
    let t = clamp(num / span, 0.0, 1.0);
    if (area) {
        return sqrt(lo * lo + (hi * hi - lo * lo) * t);
    }
    return lo + (hi - lo) * t;
}
```

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

```wgsl
// The marker quad's margin past the shape's visual radius, in pixels:
// outline strokes and their anti-aliasing reach past the contour, and
// nothing a marker draws lies outside `visual_shape_radius + QUAD_MARGIN_PX`
// on either axis.
const QUAD_MARGIN_PX: f32 = 2.0;
// Half-width of the ~1.5 px contour stroke (outline shapes and Cross).
const STROKE_HALF_PX: f32 = 0.75;

const MARKER_PI: f32 = 3.14159265359;
const MARKER_HALF_PI: f32 = 1.57079632679;
const STAR_INNER_RATIO: f32 = 0.5;

fn shape_is_filled(shape_id: u32) -> bool {
    return (shape_id >= 5u && shape_id <= 8u) || shape_id >= 17u;
}

fn base_shape_id(shape_id: u32) -> u32 {
    switch shape_id {
        case 5u: { return 0u; }   // CircleFilled
        case 6u: { return 1u; }   // SquareFilled
        case 7u: { return 2u; }   // TriangleFilled
        case 8u: { return 3u; }   // DiamondFilled
        case 9u: { return 5u; }   // TriangleDown
        case 10u: { return 6u; }  // TriangleLeft
        case 11u: { return 7u; }  // TriangleRight
        case 12u: { return 8u; }  // Plus
        case 13u: { return 9u; }  // Pentagon
        case 14u: { return 10u; } // Hexagon
        case 15u: { return 11u; } // Octagon
        case 16u: { return 12u; } // Star
        case 17u: { return 5u; }  // TriangleDownFilled
        case 18u: { return 6u; }  // TriangleLeftFilled
        case 19u: { return 7u; }  // TriangleRightFilled
        case 20u: { return 8u; }  // PlusFilled
        case 21u: { return 4u; }  // CrossFilled
        case 22u: { return 9u; }  // PentagonFilled
        case 23u: { return 10u; } // HexagonFilled
        case 24u: { return 11u; } // OctagonFilled
        case 25u: { return 12u; } // StarFilled
        default: { return shape_id; }
    }
}

// Shape-local radius/half-extent relative to the circle reference radius.
// Filled polygon markers are area-matched to a circle of radius r.
fn visual_shape_radius(base: u32, r: f32) -> f32 {
    var scale = 1.0;
    switch base {
        case 1u: { scale = 0.88622695; }  // square sqrt(pi)/2
        case 2u, 5u, 6u, 7u: { scale = 1.55512030; } // triangles
        case 3u: { scale = 1.25331414; }  // diamond sqrt(pi/2)
        case 9u: { scale = 1.14913986; }  // pentagon
        case 10u: { scale = 1.09963611; } // hexagon
        case 11u: { scale = 1.05390737; } // octagon
        case 12u: { scale = 1.46285033; } // 5-point star, inner ratio 0.5
        default: {}
    }
    return r * scale;
}

fn rotate2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

// Unsigned distance from `p` to segment `ab`. The denominator guard keeps
// a zero-length segment (radius 0) from yielding NaN — it degenerates to
// point distance instead.
fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

fn sd_box(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0);
}

fn regular_vertex(i: u32, n: u32, r: f32, rot: f32) -> vec2<f32> {
    let a = rot + f32(i) * (2.0 * MARKER_PI / f32(n));
    return vec2<f32>(cos(a), sin(a)) * r;
}

fn star_vertex(i: u32, r: f32) -> vec2<f32> {
    let outer = (i % 2u) == 0u;
    let rr = select(r * STAR_INNER_RATIO, r, outer);
    let a = MARKER_HALF_PI + f32(i) * MARKER_PI / 5.0;
    return vec2<f32>(cos(a), sin(a)) * rr;
}

fn sd_regular_polygon(p: vec2<f32>, n: u32, r: f32, rot: f32) -> f32 {
    var min_d = 1e20;
    var inside = false;
    var prev = regular_vertex(n - 1u, n, r, rot);
    for (var i = 0u; i < n; i = i + 1u) {
        let curr = regular_vertex(i, n, r, rot);
        min_d = min(min_d, sd_segment(p, prev, curr));
        if ((curr.y > p.y) != (prev.y > p.y)) {
            let x_at_y = (prev.x - curr.x) * (p.y - curr.y) / (prev.y - curr.y) + curr.x;
            if (p.x < x_at_y) {
                inside = !inside;
            }
        }
        prev = curr;
    }
    return select(min_d, -min_d, inside);
}

fn sd_star(p: vec2<f32>, r: f32) -> f32 {
    var min_d = 1e20;
    var inside = false;
    var prev = star_vertex(9u, r);
    for (var i = 0u; i < 10u; i = i + 1u) {
        let curr = star_vertex(i, r);
        min_d = min(min_d, sd_segment(p, prev, curr));
        if ((curr.y > p.y) != (prev.y > p.y)) {
            let x_at_y = (prev.x - curr.x) * (p.y - curr.y) / (prev.y - curr.y) + curr.x;
            if (p.x < x_at_y) {
                inside = !inside;
            }
        }
        prev = curr;
    }
    return select(min_d, -min_d, inside);
}

fn sd_plus_filled(p: vec2<f32>, r: f32) -> f32 {
    let arm = max(r * 0.32, STROKE_HALF_PX);
    let h = sd_box(p, vec2<f32>(r, arm));
    let v = sd_box(p, vec2<f32>(arm, r));
    return min(h, v);
}

// Exact SDF of an equilateral point-up triangle centered on the origin,
// vertices on the circle of radius `r` (same inscribed sizing as the
// diamond). `p_in` is y-up (NDC orientation), so the apex points up on
// screen. Adapted from iq's sdEquilateralTriangle.
fn sd_triangle(p_in: vec2<f32>, r: f32) -> f32 {
    let k = sqrt(3.0);
    let half_w = 0.5 * k * r; // half of the base edge
    var p = vec2<f32>(abs(p_in.x) - half_w, p_in.y + half_w / k);
    if (p.x + k * p.y > 0.0) {
        p = vec2<f32>(p.x - k * p.y, -k * p.x - p.y) * 0.5;
    }
    p.x -= clamp(p.x, -2.0 * half_w, 0.0);
    return -length(p) * sign(p.y);
}

// Signed pixel distance from `p` to the contour of `base` shape. For open
// plus/cross, the distance is unsigned to the center strokes; the outline
// alpha path below turns that into a visible stroke.
fn shape_distance(base: u32, p: vec2<f32>, r: f32, filled: bool) -> f32 {
    var d: f32;
    switch base {
        case 0u: { d = length(p) - r; }
        case 1u: { d = max(abs(p.x), abs(p.y)) - r; }
        case 2u: { d = sd_triangle(p, r); }
        case 3u: { d = abs(p.x) + abs(p.y) - r; }
        case 4u: { // Cross / x
            let c = r;
            if (filled) {
                d = sd_plus_filled(rotate2(p, MARKER_PI * 0.25), c);
            } else {
                let d1 = sd_segment(p, vec2<f32>(-c, -c), vec2<f32>(c, c));
                let d2 = sd_segment(p, vec2<f32>(-c, c), vec2<f32>(c, -c));
                d = min(d1, d2);
            }
        }
        case 5u: { d = sd_triangle(-p, r); }
        case 6u: { d = sd_triangle(rotate2(p, -MARKER_HALF_PI), r); }
        case 7u: { d = sd_triangle(rotate2(p, MARKER_HALF_PI), r); }
        case 8u: { // Plus / +
            let c = r;
            if (filled) {
                d = sd_plus_filled(p, c);
            } else {
                let d1 = sd_segment(p, vec2<f32>(-c, 0.0), vec2<f32>(c, 0.0));
                let d2 = sd_segment(p, vec2<f32>(0.0, -c), vec2<f32>(0.0, c));
                d = min(d1, d2);
            }
        }
        case 9u: { d = sd_regular_polygon(p, 5u, r, MARKER_HALF_PI); }
        case 10u: { d = sd_regular_polygon(p, 6u, r, MARKER_HALF_PI); }
        case 11u: { d = sd_regular_polygon(p, 8u, r, MARKER_HALF_PI); }
        default: { // 12u: Star
            d = sd_star(p, r);
        }
    }
    return d;
}

/// The coverage edge of a marker at shape distance `d`: a filled marker
/// covers `d < 0`; an outline (and an open plus or cross) is a stroke of
/// half-width `STROKE_HALF_PX` centred on `d == 0`. A fragment draws where
/// this is below its anti-aliasing width, and the picker's hit distance is
/// `max(marker_edge(d, filled), 0)`.
fn marker_edge(d: f32, filled: bool) -> f32 {
    return select(abs(d) - STROKE_HALF_PX, d, filled);
}
```

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

The stroke constants, `line_stroke_distance` and `dash_pattern_on` are the fragment [`data_render/common/line_stroke.wgsl`](common/line_stroke.wgsl), spliced into the line and pick modules by `wgsl_module!` (`shader_assembly.rs`). The noise is [`data_render/common/sketch_noise.wgsl`](common/sketch_noise.wgsl); `sketch_line_offset` is still copied:

```wgsl
/// The sketch line's perpendicular displacement at arc position `arc_px`:
/// `amplitude_px · noise(arc / wavelength)`. A wavelength <= 0 disables the
/// wobble — mirrors the CPU stroker's guard (sketch.rs) so GPU and deco
/// layers degrade identically.
fn sketch_line_offset(arc_px: f32, amplitude_px: f32, wavelength_px: f32, seed: u32) -> f32 {
    let wobble = max(amplitude_px, 0.0) * sketch_noise(arc_px / max(wavelength_px, 1e-6), seed);
    return select(0.0, wobble, wavelength_px > 0.0);
}
```

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

```wgsl
const SKETCH_TAU: f32 = 6.28318530718;
// Six noise periods around the contour keep marker wobble visually local.
const SKETCH_CONTOUR_WOBBLES: f32 = 6.0;
const CONSTELLATION_POINT_STAR_GAIN: f32 = 1.0;
// Ring radii relative to the planet radius, and the apparent inclination of
// the ring plane (minor/major axis ratio of the projected ellipse).
const RING_INNER: f32 = 1.45;
const RING_OUTER: f32 = 2.3;
const RING_INCL: f32 = 0.32;
// Quad half-extent multiplier — room for the ring + rim glow + AA.
const PLANET_QUAD_EXTENT: f32 = 2.5;
// A planet fragment whose composed alpha is at or below this is discarded.
const PLANET_ALPHA_FLOOR: f32 = 0.003;

/// A sketch marker's contour wobble amplitude in pixels: half the style's
/// amplitude, at most 0.35 of the marker radius.
fn sketch_marker_wobble_px(amplitude_px: f32, radius_px: f32) -> f32 {
    return min(max(amplitude_px, 0.0) * 0.5, radius_px * 0.35);
}

/// A sketch marker's signed contour distance: the precise `shape_distance`
/// perturbed with angle-parameterized noise, d' = d + k·noise(θ/τ·C), seeded
/// per marker so every point gets its own hand-drawn outline. The noise
/// lattice has a seam at θ = ±π; its magnitude is ≤ wobble_px (≤ 0.75 px at
/// default amplitude) and reads as part of the hand-drawn look.
fn sketch_marker_distance(base: u32, p: vec2<f32>, r: f32, filled: bool, wobble_px: f32, seed: u32) -> f32 {
    let theta = atan2(p.y, p.x); // [-π, π]
    return shape_distance(base, p, r, filled)
        + wobble_px * sketch_noise(theta / SKETCH_TAU * SKETCH_CONTOUR_WOBBLES, seed);
}

/// A constellation marker star's core radius in pixels: the marker radius
/// with a per-point size jitter hashed from the point index, at least half a
/// pixel. The star's quad (and its PSF sprite) spans four times this plus
/// `QUAD_MARGIN_PX`.
fn constellation_star_radius(radius_px: f32, index: u32, seed: u32) -> f32 {
    let size_jitter = 0.86 + 0.30 * sketch_hash01(index, seed ^ 0x51A7u);
    return max(radius_px * CONSTELLATION_POINT_STAR_GAIN * size_jitter, 0.5);
}

// ScatterShape code → ring position angle. The first legacy shapes get hand-
// picked angles; newer marker codes share a deterministic fallback.
fn cons_ring_angle(shape_id: u32) -> f32 {
    switch shape_id {
        case 0u: { return 0.0; }          // Circle
        case 1u: { return 0.52; }         // Square        (+30°)
        case 2u: { return 1.05; }         // Triangle      (+60°)
        case 3u: { return -0.52; }        // Diamond       (−30°)
        case 4u: { return -1.05; }        // Cross         (−60°)
        case 5u: { return 0.26; }         // CircleFilled  (+15°)
        case 6u: { return 0.79; }         // SquareFilled  (+45°)
        case 7u: { return -0.26; }        // TriangleFilled(−15°)
        case 8u: { return -0.79; }        // DiamondFilled (−45°)
        default: {
            return -1.2 + f32(shape_id % 11u) * 0.24;
        }
    }
}

/// A planet's ring position angle: the series shape's tilt plus a small
/// per-point jitter (real systems aren't aligned).
fn planet_ring_phi(shape_id: u32, seed_inst: u32) -> f32 {
    let h_jit = sketch_hash01(seed_inst, 0x717Au);
    return cons_ring_angle(shape_id) + (h_jit - 0.5) * 0.12;
}

/// `p` rotated into the ring frame of position angle `phi`.
fn planet_ring_frame(p: vec2<f32>, phi: f32) -> vec2<f32> {
    let cphi = cos(phi);
    let sphi = sin(phi);
    return vec2<f32>(p.x * cphi + p.y * sphi, -p.x * sphi + p.y * cphi);
}

/// The ring-plane radius of ring-frame point `q`, in planet radii: the ring
/// band is drawn where it lies between `RING_INNER` and `RING_OUTER`.
fn planet_ring_rho(q: vec2<f32>, r_planet: f32) -> f32 {
    return length(vec2<f32>(q.x, q.y / RING_INCL)) / r_planet;
}

/// The atmospheric rim glow's alpha at distance `dist` from the planet's
/// centre: a soft exponential falloff outside the disc.
fn planet_rim_glow(dist: f32, r_planet: f32, rim_gain: f32) -> f32 {
    let rim_d = max(dist - r_planet, 0.0);
    return rim_gain * exp(-rim_d / max(r_planet * 0.10, 1.0));
}
```

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

The items below are copied byte-for-byte into `vector/glyph_arrow.wgsl` (the
constants, structs and `arrow_*` functions), `vector/project_cartesian2d.wgsl`
(`pair_add`, `axis_px_per_unit_at`) and `gpu_pick/*.wgsl`; `shader_consistency`
compares every body, struct and constant with this document. CPU twin:
`pick.rs::vector_arrow_distance` with `arrow_place` / `arrow_distance`.

```wgsl
const ARROW_PIVOT_TAIL: u32 = 0u;
const ARROW_PIVOT_MIDDLE: u32 = 1u;
const ARROW_PIVOT_TIP: u32 = 2u;
// A shaft shorter than this (pixels) is no shaft. The head takes the whole
// length of an arrow shorter than the head; the fragment stage recomputes
// that length from the interpolated tail and tip, and the rounding must not
// turn the missing shaft into a round dot at the tail.
const ARROW_SHAFT_MIN_PX: f32 = 0.01;

/// Pixel-space arrow style. Sizes are already multiplied by the render scale.
struct ArrowStyle {
    width_px: f32,
    head_length_px: f32,
    head_width_px: f32,
    min_length_px: f32,
    pivot: u32,
};

/// An arrow placed on its anchor: tail and tip after the pivot shift, and the
/// head dimensions after scaling down for a short arrow.
struct ArrowPlacement {
    tail: vec2<f32>,
    tip: vec2<f32>,
    head_length_px: f32,
    head_width_px: f32,
    valid: bool,
};

fn arrow_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// Apply the pivot and the minimum length, and size the head to the arrow.
/// A zero-length arrow, one shorter than `min_length_px`, or one with a
/// non-finite end is not drawn.
fn arrow_place(tail: vec2<f32>, tip: vec2<f32>, style: ArrowStyle) -> ArrowPlacement {
    var out: ArrowPlacement;
    out.tail = tail;
    out.tip = tip;
    out.head_length_px = 0.0;
    out.head_width_px = 0.0;
    out.valid = false;
    let delta = tip - tail;
    let len = length(delta);
    if (!arrow_finite(len) || len <= 0.0 || len < style.min_length_px) {
        return out;
    }
    var shift = 0.0;
    if (style.pivot == ARROW_PIVOT_MIDDLE) {
        shift = -0.5;
    } else if (style.pivot == ARROW_PIVOT_TIP) {
        shift = -1.0;
    }
    out.tail = tail + delta * shift;
    out.tip = tip + delta * shift;
    let head_length = min(max(style.head_length_px, 0.0), len);
    out.head_length_px = head_length;
    if (style.head_length_px > 0.0) {
        out.head_width_px = max(style.head_width_px, 0.0) * (head_length / style.head_length_px);
    }
    out.valid = true;
    return out;
}

/// Unsigned distance from `p` to the segment `ab`; a zero-length segment is a
/// point.
fn arrow_sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

/// Signed distance from `p` to the triangle `p0 p1 p2` (negative inside).
fn arrow_sd_triangle(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / max(dot(e0, e0), 1e-12), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / max(dot(e1, e1), 1e-12), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / max(dot(e2, e2), 1e-12), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d = min(
        min(
            vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x)),
            vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x)),
        ),
        vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x)),
    );
    return -sqrt(d.x) * sign(d.y);
}

/// Signed distance from pixel `p` to the placed arrow: the union of the
/// shaft (a segment `width_px` wide) and the head (a triangle). Negative
/// inside; `1e20` for an arrow with neither.
fn arrow_distance(p: vec2<f32>, placed: ArrowPlacement, width_px: f32) -> f32 {
    let delta = placed.tip - placed.tail;
    let len = length(delta);
    let dir = delta / max(len, 1e-12);
    let nrm = vec2<f32>(-dir.y, dir.x);
    let head_base = placed.tip - dir * placed.head_length_px;
    var distance = 1e20;
    // The shaft stops where the head begins. A shaft of zero length draws
    // nothing rather than a dot.
    if (len - placed.head_length_px > ARROW_SHAFT_MIN_PX && width_px > 0.0) {
        distance = min(distance, arrow_sd_segment(p, placed.tail, head_base) - width_px * 0.5);
    }
    if (placed.head_length_px > 0.0 && placed.head_width_px > 0.0) {
        let half = nrm * (placed.head_width_px * 0.5);
        distance = min(distance, arrow_sd_triangle(p, placed.tip, head_base + half, head_base - half));
    }
    return distance;
}

/// `pair + s` as a `(hi, lo)` pair: the error-free sum keeps the low part, so a
/// large-offset coordinate still resolves a small data-space step.
fn pair_add(pair: vec2<f32>, s: f32) -> vec2<f32> {
    let sum = fma(1.0, pair.x, s);
    let b_virtual = fma(1.0, sum, -pair.x);
    let a_virtual = fma(1.0, sum, -b_virtual);
    let b_roundoff = fma(1.0, s, -b_virtual);
    let a_roundoff = fma(1.0, pair.x, -a_virtual);
    return vec2<f32>(sum, fma(1.0, a_roundoff, b_roundoff) + pair.y);
}

/// Pixels per data unit along one axis whose range is `range` (the hi/lo
/// sum), at `pair`: the derivative of `axis_pair_to_t` mapped through NDC.
/// Exact on a linear axis; the local direction on a logarithmic one, the way
/// the contour labels orient themselves.
fn axis_px_per_unit_at(range: f32, panel_scale: f32, is_log: f32, pixel_to_ndc: f32, pair: vec2<f32>) -> f32 {
    let linear = 2.0 * panel_scale / (range * pixel_to_ndc);
    // d(log10 v)/dv = 1 / (v ln 10); `sample_valid` has already excluded v <= 0.
    let value = pair.x + pair.y;
    let log_slope = linear / (value * log(10.0));
    if is_log > 0.5 {
        return log_slope;
    }
    return mix(linear, log_slope, is_log);
}
```

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
- [ ] 5.8절 화살표 글리프 함수를 바꿨다면 `vector/glyph_arrow.wgsl`,
      `vector/project_cartesian2d.wgsl`, `gpu_pick/*.wgsl`의 사본,
      `pick.rs::vector_arrow_distance` CPU 쌍둥이를 고쳤다.
- [ ] 계단 보간 헬퍼를 바꿨다면 `line_columnar.wgsl`과 `line_arc.wgsl`을 함께
      고쳤고, `gpu_pick/*.wgsl`의 `pick_step_piece_count` / `pick_step_piece`가
      같은 규칙을 따르는지 확인했다.
- [ ] `cargo test -p figgy-renderer --test shader_consistency` 통과.
- [ ] `cargo check` 통과.
- [ ] `cargo test -p figgy-renderer` 통과 (셰이더 컴파일 테스트
      `src/shader_compile_tests.rs`는 GPU 어댑터가 필요하다).
