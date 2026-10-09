<a id="config--seriesconfig--json-스키마-레퍼런스"></a>

# Config / SeriesConfig — JSON 설정 명세

기준 소스 버전: `figgy 0.10.5` / `renderer 0.12.5` / `model 0.7.2`.

이 문서는 `Config`와 `SeriesConfig`의 JSON 형식을 설명한다. figgy 0.10.0의 비상주 컬럼 등록, 구간 공급, `render_chart()` 실행·취소, 상주 가능 여부 조회는 별도 웹 API이며 설정 필드를 추가하지 않는다. 지원 범위와 호출 방법은 [WASM.md](../renderer/WASM.md#exact-streaming)의 「자동 실행과 원본 구간 공급」을 참고한다.

아래는 `FiggyChart.get_config()` / `get_series()`가 반환하고 `set_config()` / `set_series()`가 받는 JSON의 전체 형식이다. 예시 JSON은 Rust 타입을 직렬화해 생성했으며 다음 테스트로 소스와 일치하는지 확인한다.

```bash
cargo test -p model --features serde --test schema_sync
```

**설정 타입을 정의한 Rust 소스**

| 트리 | 파일 |
|---|---|
| `Config` (축·제목·격자·범례) | `crates/model/src/config.rs` |
| `SeriesConfig` (시리즈 선언) | `crates/model/src/data_config.rs` |
| `Color` | `crates/model/src/color.rs` |
| `ColorMap` (연속 색상표) | `crates/model/src/colormap.rs` |
| `RichText` / `RichSegment` | `crates/model/src/text.rs` |
| `LineStylePreset` | `crates/model/src/line.rs` |
| `LabelFormat` | `crates/model/src/format.rs` |
| `Rect` / `ChartArea` | `crates/model/src/layout/rect.rs` |

<a id="serde-표현-규칙-json을-읽을-때-알아야-할-것"></a>

## serde의 JSON 표현 규칙

- **추가 필드가 없는 열거형**은 문자열로 표시한다. 예: `"scale": "Logarithmic"`, `"tick": "Both"`.
- **데이터를 포함한 열거형**은 종류 이름을 바깥쪽 키로 쓴다(externally tagged). 예: `"render_type": { "Line": { "line": { … } } }`, `"err_x": { "Asymmetric": { "lower": "…", "upper": "…" } }`.
- **값 하나를 감싼 타입(newtype)**은 내부 값을 그대로 직렬화한다. 예: `ChartArea(Rect)` → `"chart_area": { "x": …, "width": … }`.
- **`RichSegment.text`는 문자 하나**다. JSON에서는 `"V"` 같은 한 글자 문자열로 표시한다.
- **텍스트 구간별 색·크기는 선택 항목**이다. `RichSegment.color` / `font_size`는 별도로 지정했을 때만 키를 내보낸다. 없으면 `RichText.color` / `font_size`를 따른다. 범례 기호의 색도 `{"text":"●","color":{...}}`처럼 지정한다.
- **`"\t"` 구간은 열 구분자**다. 각 열의 너비를 문서 전체에서 가장 넓은 셀에 맞추며 탭 자체는 그리지 않는다.
- **`field_em`은 기호 영역의 너비를 고정한다.** 다음 글자까지의 간격을 `field_em × 폰트 크기`로 잡고 글리프를 가운데 놓는다. `rule: true`이면 글리프 대신 그 영역을 채우는 수평선을 그린다. `rule_dash`는 선·공백 길이를 em 단위로 지정하므로 폰트 크기에 비례한다. 범례 기호는 모양에 관계없이 전체 너비가 2.0em이다. 선은 `{"text":"—","rule":true,"field_em":2.0,"color":{...}}`, 점은 `{"text":"●","field_em":2.0,...}`, 점선은 `{"text":"—","rule":true,"field_em":2.0,"rule_dash":[0.571,0.286],...}`로 표현한다. 선과 점의 조합은 선 0.65em + 글리프 0.7em + 선 0.65em으로 구성한다.
- **색은 0~1 범위의 RGBA 실수 값**이다. 예: `{ "r": 0.8, "g": 0.1, "b": 0.1, "a": 1.0 }`.
- **데이터 출처 키**인 `SeriesConfig.source_id`, `PickedPointRef.source_id`, 모든 `PickedDataRef`의 `source_id`는 `None`이면 생략한다.
- **시리즈의 스타일 매핑 키 6개**도 `None`이면 생략한다. `DataScatterStyleConfig`의 `point_style_table` / `point_style_index_column` / `point_style_overrides`와 `DataErrorBarStyleConfig`의 `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`가 해당한다.
- **개별 데이터의 스타일 키 7개**도 `None`이면 생략한다. `DataScatterPointStyleConfig`의 `point_color` / `point_shape` / `point_size`와 `DataErrorBarPointStyleConfig`의 `error_bar_color` / `error_bar_width` / `error_bar_cap_size` / `cap_width`가 해당한다. 모두 비어 있으면 `{}`로 직렬화한다. 개별 덮어쓰기 항목의 `style`은 한 단계 풀어서 저장하므로 같은 생략 규칙을 따른다.
- `SeriesConfig.label`은 위 선택 키들과 달리 항상 존재하며 값으로 `null`을 가질 수 있다.
- **등고선의 선택 키 2개**인 `ContourConfig.per_level_color` / `labels`는 `None`이면 생략한다. `per_level_color` 생략은 모든 선에 `line.line_color`를 사용한다는 뜻이며, 빈 색상 목록 `[]`과는 다르다.
- 텍스트 구간의 선택 키와 `Config`의 `draw_style` / `picked_points` / `picked_data` / `colorbar`도 생략할 수 있다. `draw_style`은 기본 정밀 모드일 때, 두 선택 표시 키는 `None`일 때 생략한다. `picked_points: {}`와 `picked_data: {}`는 각각 기본 선택 표시 설정으로 해석한다. `set_config()`는 일부 필드가 아니라 **전체 설정을 교체**하므로 `get_config()` 결과를 수정해 전달한다.

다음 직렬화 예에서는 선택 키 15개를 생략했다. `series.source_id`, 스타일 매핑 키 6개, 개별 스타일 키 7개, `picked_point.source_id`가 해당한다.

<!-- schema-sync: name=option-omissions -->
```json
{
  "series": {
    "series_id": "no-source",
    "label": null,
    "x_column": "x",
    "y_column": "y",
    "render_type": {
      "Line": {
        "line": {
          "line_style": "Solid",
          "line_color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "line_width": 1.0
        }
      }
    }
  },
  "scatter_style": {
    "point_color": {
      "r": 0.0,
      "g": 0.0,
      "b": 0.0,
      "a": 1.0
    },
    "point_shape": "CircleFilled",
    "point_size": 4.0
  },
  "errorbar_style": {
    "error_bar_color": {
      "r": 0.0,
      "g": 0.0,
      "b": 0.0,
      "a": 1.0
    },
    "error_bar_width": 1.0,
    "error_bar_cap_size": 3.0,
    "cap_width": 1.0
  },
  "scatter_point_style": {},
  "errorbar_point_style": {},
  "picked_point": {
    "series_id": "no-source",
    "point_index": 7
  }
}
```

## enum 허용값

| enum | 값 |
|---|---|
| `scale` (AxisScale) | `"Linear"` `"Logarithmic"` |
| `tick` (TickVisibility) | `"None"` `"Outside"` `"Inside"` `"Both"` |
| `format` (LabelFormat) | `"Decimal"` `"Scientific"` `"Power"` `{ "Timestamp": { ... } }` |
| `unit` (TimestampUnit) | `"Seconds"` `"Milliseconds"` `"Microseconds"` `"Nanoseconds"` |
| `timezone` (TimestampZone) | `"Utc"` `{ "FixedOffsetMinutes": 540 }` |
| `label` (TimestampLabelMode) | `"Auto"` `{ "Pattern": "%Y-%m-%d %H:%M:%S.%f" }` |
| `fractional` (FractionalSecondDigits) | `"Auto"` `{ "Fixed": 3 }` |
| `tick_policy` (TimestampTickPolicy) | `"AutoCalendar"` `"NumericSpacing"` |
| `line_style` (LineStylePreset) | `"Solid"` `"Dash"` `"Dot"` `"DashDot"` `"DashDotDot"` `"ShortDash"` `"ShortDot"` `"ShortDashDot"` `"LongDash"` `"LongDashDot"` `"LongDashDotDot"` |
| `corner` (LegendCorner) | `"TopLeft"` `"TopRight"` `"BottomLeft"` `"BottomRight"` |
| `point_shape` (ScatterShape) | `"Circle"` `"Square"` `"Triangle"` `"Diamond"` `"Cross"` `"CircleFilled"` `"SquareFilled"` `"TriangleFilled"` `"DiamondFilled"` `"TriangleDown"` `"TriangleLeft"` `"TriangleRight"` `"Plus"` `"Pentagon"` `"Hexagon"` `"Octagon"` `"Star"` `"TriangleDownFilled"` `"TriangleLeftFilled"` `"TriangleRightFilled"` `"PlusFilled"` `"CrossFilled"` `"PentagonFilled"` `"HexagonFilled"` `"OctagonFilled"` `"StarFilled"` |
| `render_type` (DataRenderType, 태그) | `"Scatter"` `"Line"` `"ScatterLine"` `"ScatterErrorbarX"` `"ScatterErrorbarY"` `"ScatterErrorbarXY"` `"LineScatterErrorbarX"` `"LineScatterErrorbarY"` `"LineScatterErrorbarXY"` `"Histogram"` `"Heatmap"` `"Contour"` `"HeatmapContour"` |
| `err_x` / `err_y` (ErrorRef, 태그) | `"Symmetric"` (`{column}`) / `"Asymmetric"` (`{lower, upper}`) |
| `side` (Side) | `"Top"` `"Bottom"` `"Left"` `"Right"` |
| `align` (BarAlign) | `"Start"` `"Center"` `"End"` |
| `colormap` (ColorMap) | `"Viridis"` `"Magma"` `"Turbo"` `"GrayScale"` `"RdBu"` `{ "Custom": { "stops": [Color, …] } }` |
| `orientation` (BarOrientation) | `"Vertical"` `"Horizontal"` |
| `orientation` (MatrixOrientation) | `"ColumnsAreX"` `"ColumnsAreY"` |
| `grid_layout` (GridLayout) | `"Edges"` `"Centers"` |
| `mode` (FillMode) | `"Continuous"` `"Bands"` |
| `shading` (Shading) | `"Flat"` `"Interpolated"` |

<a id="timestamp-label-format"></a>

### 시간 라벨 형식

시간 표시는 별도 축 스케일이 아니라 `LabelFormat`으로 설정한다. 좌표는 숫자로 유지하며 달력 기준 눈금은 선형축에서만 사용한다.

시간 라벨의 기본 설정은 다음과 같다.

```text
{
  "Timestamp": {
    "unit": "Seconds",
    "timezone": "Utc",
    "label": "Auto",
    "fractional": "Auto",
    "tick_policy": "AutoCalendar"
  }
}
```

JavaScript 타임스탬프에는 `"unit": "Milliseconds"`를 사용한다. 한국 시간처럼 고정 시차를 적용하려면 `"timezone": { "FixedOffsetMinutes": 540 }`을 지정한다.
사용자 지정 형식은 `"label": { "Pattern": "%Y-%m-%d %H:%M:%S.%f" }`처럼 쓴다. `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%f`, `%%`를 지원한다.
`AutoCalendar`는 라벨 너비를 측정하고 겹치지 않도록 달력상의 눈금 간격을 늘린다.

시간 관련 열거형 12가지의 직렬화 형식은 다음과 같다.

<!-- schema-sync: name=timestamp-variants -->
```json
{
  "unit": [
    "Seconds",
    "Milliseconds",
    "Microseconds",
    "Nanoseconds"
  ],
  "timezone": [
    "Utc",
    {
      "FixedOffsetMinutes": 540
    }
  ],
  "label": [
    "Auto",
    {
      "Pattern": "%Y-%m-%d %H:%M:%S.%f"
    }
  ],
  "fractional": [
    "Auto",
    {
      "Fixed": 3
    }
  ],
  "tick_policy": [
    "AutoCalendar",
    "NumericSpacing"
  ]
}
```

큰 Unix 시간 값은 `register_column_f64(id, Float64Array)`로 등록하고 `update_register_column_f64(id, Float64Array)`로 교체한다. GPU에 `(hi: f32, lo: f32)` 쌍으로 저장하므로 f32 하나로 표현할 수 없는 작은 시간 차이를 보존한다. 일반 좌표나 기준 시각을 뺀 상대 시간에는 `register_column_f32` / `update_register_column_f32`를 사용할 수 있다.
등록은 이미 있는 ID를 거부하고 교체는 없는 ID를 거부한다. 유효한 교체 요청은 항상 업로드한다. `set_series`는 등록된 컬럼을 지정할 뿐 데이터를 업로드하지 않는다.

[시간축 데모](timestamp-demo.html)에서 Float64Array 업로드, 시간 라벨, 자동 달력 눈금과 차트 너비·출력 배율 변경을 확인할 수 있다.

<a id="편집-시-의미-결합-주의"></a>

## 설정을 함께 바꿔야 하는 경우와 편집 규칙

- `scale`을 바꾸면 `major_spacing`의 단위도 달라진다. `Linear`는 데이터 단위이고 `Logarithmic`은 10배 간격인 decade 단위다. 예를 들어 `1.0`은 값이 10배 커질 때마다 주 눈금을 놓는다. 범위가 10배 간격보다 좁으면 주 눈금이 없을 수도 있다.
- 로그축의 `min` / `max`에는 양수를 지정한다.
- `out_margin`은 배치에 필요한 여백이다. 너무 줄이면 라벨이나 제목이 잘릴 수 있다.
- `line_offset`은 데이터 영역을 유지한 채 축만 이동시킨다. 전체 배치에는 영향을 주지 않는다.
- `inverted`는 화면상의 축 방향을 뒤집는다. 눈금·격자·데이터·피킹은 같은 변환을 적용하며 `min` / `max`는 데이터 좌표 기준을 유지한다.
- `pick_point`는 스타일 매핑을 반영한 점 기호 크기와 선 두께를 기준으로 선택한다. 선 근처를 클릭하면 해당 선분의 가까운 끝점을 반환한다. 오차 막대의 몸통과 끝선 자체는 선택 대상이 아니다. 웹 래퍼의 반환형은 `Promise<{ source_id: string | null, series_id, point_index, distance_px } | null>`이다. 저수준 `FiggyChart`는 같은 필드의 JSON 문자열 또는 `undefined`를 Promise로 반환하고 래퍼가 이를 객체 또는 `null`로 바꾼다. 요청은 제출 당시 식별자를 보관하므로 대기 중 차트·풀이 바뀌거나 렌더러가 해제돼도 다른 데이터를 가리키지 않는다. 좌표가 필요하면 `point_index`로 호스트의 원본 컬럼을 조회한다.
- `pick_data`는 점·선 외에 히스토그램 구간·히트맵 셀·등고선 레벨을 같은 GPU 요청으로 선택한다. 래퍼는 `kind`가 `"point" | "histogram_bin" | "matrix_cell" | "contour_level"`인 객체 또는 `null`을 반환한다. 구간은 `bin_index`, 셀은 X·Y축 기준 `x_index` / `y_index`, 등고선은 `level_index`와 선택 지점이 속한 표본 셀의 `x_index` / `y_index`를 제공한다. `HeatmapContour`에서 선과 셀이 겹치면 나중에 그리는 등고선이 우선하며 시리즈 간 동률은 그리기 순서로 정한다. 좌표·경계·선분·f64 값을 CPU에서 복원하지 않고, 그리기와 같은 GPU 변환·풀·격자·등고선 함수로 판정한다.
- `chart_area`는 저장과 출력에 사용하는 문서 영역이다. 웹의 `resize(w, h)`는 캔버스만 바꾸며 문서를 같은 가로세로 비율로 맞추고 남는 공간에 여백을 둔다. 브라우저 창 크기 변경은 `chart_area` 편집과 구분한다.
- `set_series` / `apply_color_cycle`은 자동 범례에서 인식할 수 있는 기호 부분만 갱신한다. `\t` 앞의 고정 너비 영역이 기호이며, 뒤의 사용자 텍스트는 보존한다. 선 색뿐 아니라 `line_style`의 점선·도트 패턴도 반영한다.
- `set_series_label(id, label)`은 해당 범례의 텍스트만 바꾼다. 빈 문자열이면 해당 행을 제거한다. `set_config`로 직접 편집한 `legend.content`는 이후 시리즈 변경으로 전체를 다시 쓰지 않는다.
- 전체 범례를 `SeriesConfig.label`에서 다시 만들 때는 `reset_legend_from_series_labels()`를 호출한다. 기존 `add_line_series(..., label)`로 저장한 라벨은 리치 텍스트 라벨이 없는 시리즈에서만 대체값으로 사용한다.

<a id="picked_data--typed-data-selection-overlay-config-선택-키"></a>

## `picked_data` — 데이터 종류별 선택 표시

`Config.picked_data`는 `pick_data` 결과의 식별자만 보관한다. `set_picked_data(json)`은 이 필드만 교체하고 JSON `null`이면 표시를 지운다. 점 전용 `picked_points`와 독립적이므로 둘을 함께 사용할 수 있다.

```json
{
  "picked_data": {
    "visible": true,
    "refs": [
      { "kind": "point", "series_id": "points", "point_index": 4 },
      { "kind": "histogram_bin", "series_id": "hist", "bin_index": 2 },
      { "kind": "matrix_cell", "series_id": "heat", "x_index": 3, "y_index": 1 },
      {
        "kind": "contour_level",
        "series_id": "contour",
        "level_index": 5,
        "x_index": 3,
        "y_index": 1
      }
    ],
    "highlight_color": { "r": 1.0, "g": 0.84313726, "b": 0.0, "a": 1.0 },
    "outline_width_px": 2.0,
    "point_radius_extra_px": 3.0,
    "contour_width_extra_px": 2.0
  }
}
```

각 참조에 `source_id`를 넣으면 같은 `series_id`를 가진 서로 다른 데이터 출처를 구분할 수 있다. 점은 기존 강조 테두리 렌더링을 사용한다. 히스토그램은 선택한 막대의 경계·값·스타일 바인드 그룹을, 행렬·등고선은 일반 그리기와 같은 행렬 바인드 그룹을 읽는다. 축이나 데이터를 갱신해도 선택 표시가 데이터 위치를 따라가며, 범위를 벗어난 인덱스는 그리지 않는다.

<a id="draw_style--렌더-스타일-config-선택-키"></a>

## `draw_style` — 렌더링 스타일

`Config.draw_style`은 `crates/model/src/config.rs`의 `DrawStyle` 열거형이다. `"mode"`와 해당 스타일의 옵션을 같은 객체 안에 저장한다(internally tagged). 키를 생략하면 기본 정밀 모드인 `precise`다. 기본값을 직렬화할 때도 키를 생략하지만 `{ "mode": "precise" }`를 직접 지정해도 된다.
`{ "mode": "sketch" }`는 손그림 스타일을 켠다. 스타일은 차트 전체에 적용하며 시리즈마다 다르게 지정할 수 없다.

스케치 옵션에는 모두 기본값이 있어 필요한 항목만 지정할 수 있다. `"draw_style": { "mode": "sketch" }`만 넣으면 모든 옵션에 기본값을 사용한다.

| 필드 (`mode: "sketch"`) | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `amplitude_px` | f32 | `1.5` | 선에 수직인 방향으로 흔들리는 폭(px) |
| `wavelength_px` | f32 | `60.0` | 선을 따라 한 번 굽이치는 간격(px) |
| `seed` | u32 | `0` | 같은 설정·데이터에서 같은 패턴을 만드는 시드 |

전체 형태: `"draw_style": { "mode": "sketch", "amplitude_px": 1.5,
"wavelength_px": 60.0, "seed": 0 }` — 정밀 모드로 되돌리려면 키를
제거한다(또는 `{ "mode": "precise" }`).

은하수 스타일도 `draw_style`로 선택한다. 모든 옵션에 기본값이 있으므로 `"draw_style": { "mode": "milkyway" }`만 지정해 사용할 수 있다.

| 필드 (`mode: "milkyway"`) | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `star_density` | f32 | `14.0` | 경로 길이 100px당 별의 밀도 |
| `ribbon_width_px` | f32 | `14.0` | 시리즈 색으로 그리는 성운 띠의 너비 |
| `ribbon_intensity` | f32 | `0.30` | 성운 띠의 밝기 |
| `star_scale` | f32 | `1.0` | 별 크기 배율 |
| `star_brightness` | f32 | `1.0` | 별 밝기 배율 |
| `spread_px` | f32 | `2.5` | 별이 경로 주위로 퍼지는 정도 |
| `structure_scale` | f32 | `1.0` | 별 무리 등 구조의 크기 배율 |
| `faint_bias` | f32 | `3.0` | 어두운 별의 비중을 높이는 정도 |
| `glow` | f32 | `0.55` | 축·배경의 빛 번짐 강도 |
| `nebula` | f32 | `1.0` | 배경 성운 강도 |
| `dust` | f32 | `1.0` | 배경 먼지 밀도 |
| `planet_rim` | f32 | `0.34` | 산점도 행성의 가장자리 빛 강도 |
| `seed` | u32 | `0` | 전역 시드 |

전체 형태: `"draw_style": { "mode": "milkyway", "star_density": 14.0,
"ribbon_width_px": 14.0, "ribbon_intensity": 0.30, "star_scale": 1.0,
"star_brightness": 1.0, "spread_px": 2.5, "structure_scale": 1.0,
"faint_bias": 3.0, "glow": 0.55, "nebula": 1.0, "dust": 1.0,
"planet_rim": 0.34, "seed": 0 }`.

별자리 스타일인 `constellation`은 `ScatterLine` 시리즈만 지원한다. 데이터 점 위치에 PSF로 별을 그리고 반투명 선으로 잇는다.

| 필드 (`mode: "constellation"`) | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `star_opacity` | f32 | `1.0` | 별의 불투명도 |
| `line_opacity` | f32 | `0.45` | 연결선의 불투명도 |

전체 형태: `"draw_style": { "mode": "constellation", "star_opacity": 1.0,
"line_opacity": 0.45 }`.

## `get_config()` 전체 형태 — 기본값 기준

기본 `Config`의 `draw_style`은 `Precise`, `picked_points`와 `picked_data`는 `None`이므로 JSON에서 이 세 키를 생략한다. 선택 표시를 사용하려면 해당 객체를 추가한다. 빈 객체 `{}`는 각각의 기본 표시 설정으로 해석한다.

<!-- schema-sync: name=config -->
```json
{
  "chart_area": {
    "x": 0,
    "y": 0,
    "width": 1000,
    "height": 800
  },
  "top_x": {
    "scale": "Linear",
    "min": 0.0,
    "max": 1.0,
    "major_spacing": 0.2,
    "minor_count": 4,
    "inverted": false,
    "label_style": {
      "visible": true,
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 18.0,
      "label_visible": false,
      "label_font": "",
      "label_offset_x": 0.0,
      "label_offset_y": 0.0,
      "format": "Decimal",
      "significant_digits": 3
    },
    "tick": "Inside",
    "title_option": {
      "text": {
        "segments": [],
        "color": {
          "r": 0.0,
          "g": 0.0,
          "b": 0.0,
          "a": 1.0
        },
        "font_size": 22.0,
        "font": ""
      },
      "visible": false,
      "offset_x": 0.0,
      "offset_y": 0.0
    },
    "out_margin": 8.0,
    "line_offset": 0.0,
    "line_visible": true,
    "line_color": {
      "r": 0.0,
      "g": 0.0,
      "b": 0.0,
      "a": 1.0
    },
    "line_width": 1.0,
    "line_style": "Solid",
    "major_tick_length": 5.0,
    "minor_tick_length": 3.0
  },
  "bottom_x": {
    "scale": "Linear",
    "min": 0.0,
    "max": 1.0,
    "major_spacing": 0.2,
    "minor_count": 4,
    "inverted": false,
    "label_style": {
      "visible": true,
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 18.0,
      "label_visible": true,
      "label_font": "",
      "label_offset_x": 0.0,
      "label_offset_y": 0.0,
      "format": "Decimal",
      "significant_digits": 3
    },
    "tick": "Inside",
    "title_option": {
      "text": {
        "segments": [],
        "color": {
          "r": 0.0,
          "g": 0.0,
          "b": 0.0,
          "a": 1.0
        },
        "font_size": 22.0,
        "font": ""
      },
      "visible": true,
      "offset_x": 0.0,
      "offset_y": 0.0
    },
    "out_margin": 80.0,
    "line_offset": 0.0,
    "line_visible": true,
    "line_color": {
      "r": 0.0,
      "g": 0.0,
      "b": 0.0,
      "a": 1.0
    },
    "line_width": 1.0,
    "line_style": "Solid",
    "major_tick_length": 5.0,
    "minor_tick_length": 3.0
  },
  "left_y": {
    "scale": "Linear",
    "min": 0.0,
    "max": 1.0,
    "major_spacing": 0.2,
    "minor_count": 4,
    "inverted": false,
    "label_style": {
      "visible": true,
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 18.0,
      "label_visible": true,
      "label_font": "",
      "label_offset_x": 0.0,
      "label_offset_y": 0.0,
      "format": "Decimal",
      "significant_digits": 3
    },
    "tick": "Inside",
    "title_option": {
      "text": {
        "segments": [],
        "color": {
          "r": 0.0,
          "g": 0.0,
          "b": 0.0,
          "a": 1.0
        },
        "font_size": 22.0,
        "font": ""
      },
      "visible": true,
      "offset_x": 0.0,
      "offset_y": 0.0
    },
    "out_margin": 110.0,
    "line_offset": 0.0,
    "line_visible": true,
    "line_color": {
      "r": 0.0,
      "g": 0.0,
      "b": 0.0,
      "a": 1.0
    },
    "line_width": 1.0,
    "line_style": "Solid",
    "major_tick_length": 5.0,
    "minor_tick_length": 3.0
  },
  "right_y": {
    "scale": "Linear",
    "min": 0.0,
    "max": 1.0,
    "major_spacing": 0.2,
    "minor_count": 4,
    "inverted": false,
    "label_style": {
      "visible": true,
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 18.0,
      "label_visible": false,
      "label_font": "",
      "label_offset_x": 0.0,
      "label_offset_y": 0.0,
      "format": "Decimal",
      "significant_digits": 3
    },
    "tick": "Inside",
    "title_option": {
      "text": {
        "segments": [],
        "color": {
          "r": 0.0,
          "g": 0.0,
          "b": 0.0,
          "a": 1.0
        },
        "font_size": 22.0,
        "font": ""
      },
      "visible": false,
      "offset_x": 0.0,
      "offset_y": 0.0
    },
    "out_margin": 8.0,
    "line_offset": 0.0,
    "line_visible": true,
    "line_color": {
      "r": 0.0,
      "g": 0.0,
      "b": 0.0,
      "a": 1.0
    },
    "line_width": 1.0,
    "line_style": "Solid",
    "major_tick_length": 5.0,
    "minor_tick_length": 3.0
  },
  "chart_title": {
    "text": {
      "segments": [],
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 28.0,
      "font": ""
    },
    "visible": true,
    "offset_x": 0.0,
    "offset_y": 0.0,
    "top_margin": 32.0
  },
  "grid": {
    "show_major_x": true,
    "major_x_color": {
      "r": 0.78431374,
      "g": 0.78431374,
      "b": 0.78431374,
      "a": 1.0
    },
    "major_x_width": 1.0,
    "major_x_style": "Solid",
    "show_major_y": true,
    "major_y_color": {
      "r": 0.78431374,
      "g": 0.78431374,
      "b": 0.78431374,
      "a": 1.0
    },
    "major_y_width": 1.0,
    "major_y_style": "Solid",
    "show_minor_x": false,
    "minor_x_color": {
      "r": 0.9019608,
      "g": 0.9019608,
      "b": 0.9019608,
      "a": 1.0
    },
    "minor_x_width": 0.5,
    "minor_x_style": "Dot",
    "show_minor_y": false,
    "minor_y_color": {
      "r": 0.9019608,
      "g": 0.9019608,
      "b": 0.9019608,
      "a": 1.0
    },
    "minor_y_width": 0.5,
    "minor_y_style": "Dot"
  },
  "legend": {
    "visible": false,
    "content": {
      "segments": [],
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 14.0,
      "font": ""
    },
    "corner": "TopRight",
    "offset_x": 0.0,
    "offset_y": 0.0,
    "padding": 8.0,
    "bg_color": {
      "r": 1.0,
      "g": 1.0,
      "b": 1.0,
      "a": 0.85
    },
    "border_color": {
      "r": 0.6,
      "g": 0.6,
      "b": 0.6,
      "a": 1.0
    }
  }
}
```

<a id="colorbar--컬러바와-z-스케일-config-선택-키"></a>

## `colorbar` — 색상 막대와 z 스케일 (Config 선택 키)

`Config.colorbar`는 색상 막대의 모양과 차트의 Z축 범위를 함께 정의한다. 기본 설정에는 없으며 `None`일 때 키를 생략한다. 따라서 Z축이 필요 없는 기존 문서도 그대로 읽을 수 있다.

`axis`는 차트의 네 축과 같은 `AxisOptions`다. 스케일·최소·최대·눈금 간격·라벨·제목의 의미가 같으며, 로그축과 `Power` 라벨을 포함한 눈금 생성·표시를 같은 코드로 처리한다.

| 필드 | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `visible` | bool | `true` | `false`이면 색상 막대와 여백을 없앤다. 행렬은 계속 그린다. |
| `side` | Side | `"Right"` | `Left`/`Right` = 수직 바, `Top`/`Bottom` = 수평 바. 이것만으로 방향이 결정된다 |
| `thickness_px` | f32 | `18.0` | 색상 막대 두께 |
| `gap_px` | f32 | `24.0` | 색상 막대의 배치 간격(px) |
| `length_frac` | f32 | `0.75` | 배치한 변의 길이에 대한 막대 길이 비율. `(0, 1]` |
| `align` | BarAlign | `"Center"` | 변을 따라 시작·가운데·끝 중 어디에 놓을지 지정 |
| `offset_x`, `offset_y` | f32 | `0.0` | 기준 위치에서의 이동량(px). 드래그 결과를 누적한다. 여백 계산에 반영하지 않으므로 데이터 영역은 그대로다. |
| `colormap` | ColorMap | `"Viridis"` | 값에 따라 연속적으로 색을 지정하는 색상표 |
| `nan_color` | Color | 완전 투명 | 색으로 변환할 수 없는 Z값(NaN, 로그 스케일의 0 이하 값)에 사용할 색 |
| `border_color` | Color | 회색(80,80,80) | 색상 막대 테두리 |
| `border_width` | f32 | `1.0` | 〃 |
| `axis` | AxisOptions | 아래 | Z축 범위와 눈금·라벨 설정 |

색상 막대는 다음 규칙을 따른다.

- `Heatmap` / `Contour` / `HeatmapContour`에는 이 설정이 반드시 필요하다. Z축 범위와 색상표를 여기에서 읽으므로 없으면 시리즈를 거부한다.
- 차트당 Z축 스케일은 하나이며 여러 히트맵이 같은 스케일을 공유한다.
- 색상 막대의 영역은 `gap_px + thickness_px + axis.out_margin + axis.major_tick_length`이며 해당 방향의 여백에 추가된다. `fit` / `resize`는 라벨 여백인 `axis.out_margin`만 조절한다. `thickness_px`, `gap_px`, 눈금 길이는 유지한다.
- 일반 축과 다른 기본값은 `line_visible: false`와 `tick: "Outside"`다. 막대 테두리가 축선 역할을 하고 눈금은 라벨 공간에 놓인다. `tick`은 `None` / `Outside` / `Inside` / `Both`로 눈금 방향을, `inverted`는 화면상의 값 증가 방향을 정한다. 눈금 모양은 축선의 `line_color` / `line_width` / `line_style`을 따른다.
- 색상 막대는 선택·이동·크기 조절을 지원한다. 히트테스트 ID는 `"colorbar"`다. 선택하면 데이터 영역과 마찬가지로 파란 상자와 조절점 8개를 표시한다. 이동량은 `offset_{x,y}`에 누적한다. 크기 조절은 막대 방향에 따라 짧은 쪽의 `thickness_px` 또는 긴 쪽의 `length_frac`을 바꾼다.
- 축·눈금 라벨·제목은 `"colorbar_axis"`, `"colorbar_tick_labels"`, `"colorbar_title"`로 각각 선택한다. 드래그하면 `axis.line_offset`, `axis.label_style.label_offset_{x,y}`, `axis.title_option.offset_{x,y}`가 바뀐다. 위치와 선택 영역은 실제 막대 사각형을 기준으로 계산하므로 길이·정렬·위치·크기 변경을 함께 반영한다.
- 웹에서는 `set_colorbar_axis(json)`으로 전체 축 설정을 바꾸거나 `set_colorbar_title(text)`로 제목을 지정한다. 빈 제목은 숨긴다. 두 호출 모두 색상 막대가 없으면 실패한다.

<!-- schema-sync: name=colorbar -->
```json
{
  "visible": true,
  "side": "Right",
  "thickness_px": 18.0,
  "gap_px": 24.0,
  "length_frac": 0.75,
  "align": "Center",
  "offset_x": 0.0,
  "offset_y": 0.0,
  "colormap": "Viridis",
  "nan_color": {
    "r": 0.0,
    "g": 0.0,
    "b": 0.0,
    "a": 0.0
  },
  "border_color": {
    "r": 0.3137255,
    "g": 0.3137255,
    "b": 0.3137255,
    "a": 1.0
  },
  "border_width": 1.0,
  "axis": {
    "scale": "Linear",
    "min": 0.0,
    "max": 1.0,
    "major_spacing": 0.2,
    "minor_count": 4,
    "inverted": false,
    "label_style": {
      "visible": true,
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 18.0,
      "label_visible": true,
      "label_font": "",
      "label_offset_x": 0.0,
      "label_offset_y": 0.0,
      "format": "Decimal",
      "significant_digits": 3
    },
    "tick": "Outside",
    "title_option": {
      "text": {
        "segments": [],
        "color": {
          "r": 0.0,
          "g": 0.0,
          "b": 0.0,
          "a": 1.0
        },
        "font_size": 22.0,
        "font": ""
      },
      "visible": false,
      "offset_x": 0.0,
      "offset_y": 0.0
    },
    "out_margin": 60.0,
    "line_offset": 0.0,
    "line_visible": false,
    "line_color": {
      "r": 0.0,
      "g": 0.0,
      "b": 0.0,
      "a": 1.0
    },
    "line_width": 1.0,
    "line_style": "Solid",
    "major_tick_length": 5.0,
    "minor_tick_length": 3.0
  }
}
```

<a id="면--막대-render_type--전체-형태"></a>

## 행렬·막대 render_type의 전체 형식

히스토그램과 행렬 기반 세 종류의 전체 JSON 예시는 아래와 같다. `Histogram`은 호스트가 구간별로 집계한 `(edges, counts)` 컬럼을 받고, 행렬은 `MatrixRef`로 격자를 지정한다.

- `Histogram`의 컬럼 역할은 `bar.orientation`으로 정한다. `"Vertical"`이면 `x_column`이 경계, `y_column`이 빈도 값이며 `"Horizontal"`은 반대다. `edges = counts + 1` 같은 길이 관계로 추측하지 않는다.
- `bar.width_ratio`는 구간 대비 막대 너비이며 `0..=1`로 제한한다. 막대를 가운데에 놓고 `gap_px`를 양쪽으로 나누어 추가 간격을 둔다. 1픽셀보다 넓은 막대는 최소 1픽셀이 남도록 간격을 제한한다.
- 구간 자체가 1픽셀보다 좁으면 GPU가 픽셀 열별 최댓값을 골라 0까지 채운다. 가로 히스토그램은 픽셀 행을 기준으로 한다. 이때 간격과 양수인 너비 비율은 적용하지 않는다. 최댓값 구간의 외곽선 두께와 불투명도가 모두 양수이면 외곽선 색으로, 아니면 채움색으로 그린다. 동률이면 앞선 구간을 선택한다. `width_ratio: 0`인 구간은 제외하며 원본 컬럼은 바꾸지 않는다.
- `border_width: 0`이면 외곽선을 숨긴다. 양수이면 지정한 두께와 `border_color`를 사용한다.
- `bar.bar_style_overrides`는 `index`로 구간을 골라 `fill_color`, `border_color`, `border_width`, `gap_px`, `width_ratio` 중 필요한 값만 바꾼다. 같은 인덱스가 여러 번 나오면 선언 순서대로 적용한다. 기준선과 방향은 시리즈 전체에 적용한다. 그리기·데이터 피킹·선택 테두리는 같은 최종 막대 경계를 사용한다.
- `MatrixRef.columns`는 격자를 구성하는 컬럼 ID 목록이다. 별도의 데이터 객체를 만들지 않고 등록된 컬럼을 사용한다.
- `MatrixRef.grid_layout`은 좌표가 셀 경계(`"Edges"`, n+1개)인지 중심(`"Centers"`, n개)인지 지정한다. 길이로 추측하지 않는다.
- 선언과 데이터 크기가 다르면 공통으로 사용할 수 있는 범위까지만 그리고 잘림 여부를 알린다.
<!-- contour-contract: scope=schema max-levels=1024 -->
- `ContourConfig.levels`에는 데이터 단위의 등고선 값을 명시한다. 자동 추론 옵션은 없다. `0..=1024`개를 허용하며 1025개 이상이면 `set_series`가 실패한다. 배열을 잘라 처리하지 않고 이전 설정·시리즈·GPU 스타일을 유지한다.
- `ContourLabelConfig.spacing_px`는 자동 배치의 목표 간격이다. 레벨 누락을 막기 위해 추가로 고르는 후보는 더 가까울 수 있다. 라벨을 숨겼거나 위치를 직접 지정했더라도 간격은 유한한 양수여야 한다.
- 자동·직접 배치는 모두 최대 1024개를 사용한다. 직접 지정한 `anchors`에서는 잘못된 `level_index`를 제외하고 입력 순서대로 유효한 앞 1024개를 사용한다. 남은 항목이 없으면 자동 배치하고, 하나라도 있으면 그 목록을 사용한다.
- 자동 배치에서는 허용 범위로 제한한 프레임·출력 배율을 간격에 곱하고 결과가 유한한 양수인지 다시 검사한다. 아틀라스는 WebGPU 어댑터의 텍스처 크기 한도를 지켜야 한다. 검증에 실패하면 이전 설정·시리즈·GPU 스타일을 유지한다. 앵커는 데이터 좌표 `(x, y)`와 접선 `(tx, ty)`으로 저장하며 확대·이동 시 화면에 다시 투영한다.
- `ContourLabelConfig.color`는 선 색과 `per_level_color`에 영향을 받지 않는 라벨 글자색이다. 소수 표시는 등고선 간격(없으면 색상 막대 간격)과 `significant_digits`를 사용한다. 선은 실제 라벨 사각형 안에서 끊으며 `bg_padding_px`만큼 추가로 비운다. 배경색이 없어도 이 간격을 적용한다.

<!-- schema-sync: name=field-render-types -->
```json
{
  "histogram": {
    "Histogram": {
      "bar": {
        "fill_color": {
          "r": 0.27450982,
          "g": 0.50980395,
          "b": 0.7058824,
          "a": 1.0
        },
        "border_color": {
          "r": 0.0,
          "g": 0.0,
          "b": 0.0,
          "a": 1.0
        },
        "border_width": 1.0,
        "baseline": 0.0,
        "gap_px": 1.0,
        "width_ratio": 0.85,
        "orientation": "Vertical",
        "bar_style_overrides": [
          {
            "index": 1,
            "fill_color": {
              "r": 0.9019608,
              "g": 0.22352941,
              "b": 0.27450982,
              "a": 1.0
            },
            "border_color": {
              "r": 0.47058824,
              "g": 0.078431375,
              "b": 0.11764706,
              "a": 1.0
            },
            "border_width": 2.0,
            "width_ratio": 0.6
          }
        ]
      }
    }
  },
  "heatmap": {
    "Heatmap": {
      "matrix": {
        "columns": [
          "z0",
          "z1",
          "z2"
        ],
        "orientation": "ColumnsAreX",
        "grid_layout": "Edges"
      },
      "fill": {
        "mode": "Continuous",
        "shading": "Interpolated",
        "opacity": 1.0
      }
    }
  },
  "contour": {
    "Contour": {
      "matrix": {
        "columns": [
          "z0",
          "z1",
          "z2"
        ],
        "orientation": "ColumnsAreX",
        "grid_layout": "Edges"
      },
      "contour": {
        "levels": [
          1.0,
          2.0,
          5.0
        ],
        "line": {
          "line_style": "Solid",
          "line_color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "line_width": 1.0
        },
        "per_level_color": [
          {
            "r": 0.9019608,
            "g": 0.22352941,
            "b": 0.27450982,
            "a": 1.0
          },
          {
            "r": 0.11372549,
            "g": 0.20784314,
            "b": 0.34117648,
            "a": 1.0
          },
          {
            "r": 0.16470589,
            "g": 0.6156863,
            "b": 0.56078434,
            "a": 1.0
          }
        ],
        "labels": {
          "visible": true,
          "font_size": 12.0,
          "color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "format": "Decimal",
          "significant_digits": 3,
          "spacing_px": 140.0,
          "anchors": [
            {
              "level_index": 1,
              "x": 0.5,
              "y": 0.25,
              "tx": 1.0,
              "ty": 0.0
            }
          ],
          "bg_color": {
            "r": 1.0,
            "g": 1.0,
            "b": 1.0,
            "a": 1.0
          },
          "bg_padding_px": 2.0
        }
      }
    }
  },
  "heatmap_contour": {
    "HeatmapContour": {
      "matrix": {
        "columns": [
          "z0",
          "z1",
          "z2"
        ],
        "orientation": "ColumnsAreX",
        "grid_layout": "Edges"
      },
      "fill": {
        "mode": "Continuous",
        "shading": "Interpolated",
        "opacity": 1.0
      },
      "contour": {
        "levels": [
          1.0,
          2.0,
          5.0
        ],
        "line": {
          "line_style": "Solid",
          "line_color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "line_width": 1.0
        },
        "per_level_color": [
          {
            "r": 0.9019608,
            "g": 0.22352941,
            "b": 0.27450982,
            "a": 1.0
          },
          {
            "r": 0.11372549,
            "g": 0.20784314,
            "b": 0.34117648,
            "a": 1.0
          },
          {
            "r": 0.16470589,
            "g": 0.6156863,
            "b": 0.56078434,
            "a": 1.0
          }
        ],
        "labels": {
          "visible": true,
          "font_size": 12.0,
          "color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "format": "Decimal",
          "significant_digits": 3,
          "spacing_px": 140.0,
          "anchors": [
            {
              "level_index": 1,
              "x": 0.5,
              "y": 0.25,
              "tx": 1.0,
              "ty": 0.0
            }
          ],
          "bg_color": {
            "r": 1.0,
            "g": 1.0,
            "b": 1.0,
            "a": 1.0
          },
          "bg_padding_px": 2.0
        }
      }
    }
  }
}
```

<a id="get_series-전체-형태--최대-변형-예시"></a>

## `get_series()` 전체 형식 — 모든 오차 방향을 포함한 예

`LineScatterErrorbarXY`, 대칭·비대칭 `ErrorRef`, 라벨을 모두 포함한 시리즈 하나의 예다. 다른 시리즈 종류는 해당 종류에 필요한 필드만 사용한다.

<!-- schema-sync: name=series -->
```json
[
  {
    "series_id": "example",
    "source_id": "source-a",
    "label": {
      "segments": [
        {
          "text": "V",
          "bold": false,
          "italic": false,
          "underline": false,
          "superscript": false,
          "subscript": false,
          "greek": false
        },
        {
          "text": "0",
          "bold": false,
          "italic": false,
          "underline": false,
          "superscript": false,
          "subscript": true,
          "greek": false
        }
      ],
      "color": {
        "r": 0.0,
        "g": 0.0,
        "b": 0.0,
        "a": 1.0
      },
      "font_size": 14.0,
      "font": ""
    },
    "x_column": "x",
    "y_column": "y",
    "render_type": {
      "LineScatterErrorbarXY": {
        "scatter": {
          "point_color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "point_shape": "CircleFilled",
          "point_size": 4.0,
          "point_style_table": [
            {
              "point_color": {
                "r": 0.9019608,
                "g": 0.22352941,
                "b": 0.27450982,
                "a": 1.0
              },
              "point_shape": "CircleFilled",
              "point_size": 5.0
            },
            {
              "point_color": {
                "r": 0.11372549,
                "g": 0.20784314,
                "b": 0.34117648,
                "a": 1.0
              },
              "point_shape": "DiamondFilled"
            }
          ],
          "point_style_index_column": "style_index",
          "point_style_overrides": [
            {
              "index": 3,
              "point_shape": "StarFilled",
              "point_size": 7.0
            }
          ]
        },
        "line": {
          "line_style": "Solid",
          "line_color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "line_width": 2.0
        },
        "err_x": {
          "Asymmetric": {
            "lower": "ex_lo",
            "upper": "ex_hi"
          }
        },
        "err_y": {
          "Symmetric": {
            "column": "ey"
          }
        },
        "err_style": {
          "error_bar_color": {
            "r": 0.0,
            "g": 0.0,
            "b": 0.0,
            "a": 1.0
          },
          "error_bar_width": 1.0,
          "error_bar_cap_size": 3.0,
          "cap_width": 1.0,
          "error_bar_style_table": [
            {
              "error_bar_color": {
                "r": 0.8509804,
                "g": 0.14117648,
                "b": 0.14117648,
                "a": 1.0
              },
              "error_bar_width": 2.0
            },
            {
              "error_bar_cap_size": 6.0,
              "cap_width": 2.0
            }
          ],
          "error_bar_style_index_column": "err_style_index",
          "error_bar_style_overrides": [
            {
              "index": 2,
              "error_bar_color": {
                "r": 0.11372549,
                "g": 0.20784314,
                "b": 0.34117648,
                "a": 1.0
              },
              "cap_width": 3.0
            }
          ]
        }
      }
    }
  }
]
```

오차 막대와 산점도의 스타일 매핑은 독립적이다. `error_bar_style_index_column`은 `error_bar_style_table`의 행을 선택하며, `error_bar_style_overrides`는 원본 점 인덱스로 특정 값만 덮어쓴다. 정밀 모드에서 색·몸통 두께·끝선 절반 길이·끝선 두께에 적용하며 다른 렌더링 스타일은 이 매핑을 무시한다.
