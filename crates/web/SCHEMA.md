# Config / SeriesConfig — JSON 스키마 레퍼런스

적용 공개 버전: `figgy 0.9.1` / `renderer 0.11.0`.

`FiggyChart.get_config()` / `get_series()`가 반환하고 `set_config()` /
`set_series()`가 받는 JSON의 **전체 형태**다. 아래 JSON 블록은 Rust 소스에서
직접 직렬화해 생성했고, 동기화 테스트가 어긋남을 막는다:

```bash
cargo test -p model --features serde --test schema_sync
```

**타입의 진실 원본 (Rust 소스)**

| 트리 | 파일 |
|---|---|
| `Config` (축/타이틀/그리드/범례) | `crates/model/src/config.rs` |
| `SeriesConfig` (시리즈 선언) | `crates/model/src/data_config.rs` |
| `Color` | `crates/model/src/color.rs` |
| `ColorMap` (연속 컬러 램프) | `crates/model/src/colormap.rs` |
| `RichText` / `RichSegment` | `crates/model/src/text.rs` |
| `LineStylePreset` | `crates/model/src/line.rs` |
| `LabelFormat` | `crates/model/src/format.rs` |
| `Rect` / `ChartArea` | `crates/model/src/layout/rect.rs` |

## serde 표현 규칙 (JSON을 읽을 때 알아야 할 것)

- **필드 없는 enum은 문자열**: `"scale": "Logarithmic"`, `"tick": "Both"`.
- **데이터를 가진 enum은 externally-tagged 객체**:
  `"render_type": { "Line": { "line": { … } } }`,
  `"err_x": { "Asymmetric": { "lower": "…", "upper": "…" } }`.
- **newtype은 내용물로 평탄화**: `ChartArea(Rect)` → `"chart_area": { "x": …, "width": … }`.
- **`RichSegment.text`는 char** → JSON에서 글자 1개짜리 문자열 `"V"`.
- **세그먼트별 오버라이드는 선택 키**: `RichSegment`의 `color` / `font_size`
  는 오버라이드가 있을 때만 직렬화된다 (없으면 키 자체가 생략 → 문서
  레벨 `RichText.color` / `font_size` 상속). 범례 심볼이 이 방식으로
  시리즈 색을 갖는다: `{"text":"●","color":{...}}`.
- **`"\t"` 세그먼트 = 열 구분자**: 표처럼 각 열 폭이 문서 전체에서 가장
  넓은 셀에 맞춰진다. 탭 자체는 렌더되지 않는다.
- **고정폭 심볼 필드**: 세그먼트의 `field_em` (선택 키) 은 글리프 폭과
  무관하게 advance 를 `field_em × 폰트크기` 로 고정하고 잉크를 필드
  중앙에 둔다. `rule: true` (선택 키) 는 글리프 대신 필드 전체를 채우는
  **그려진 수평선**이다. `rule_dash` (선택 키) 는 rule 전용 dash/gap
  패턴이며 em 단위라 폰트 크기와 함께 스케일된다. 범례 심볼은 이
  조합으로 어떤 형태든 정확히 같은 길이(2.0 em)가 된다: 선 =
  `{"text":"—","rule":true,"field_em":2.0,"color":{...}}`, 점 =
  `{"text":"●","field_em":2.0,...}`, 점선 =
  `{"text":"—","rule":true,"field_em":2.0,"rule_dash":[0.571,0.286],...}`,
  선+점 = rule(0.65) + 글리프(0.7) + rule(0.65).
- **색은 0..1 float RGBA**: `{ "r": 0.8, "g": 0.1, "b": 0.1, "a": 1.0 }`.
- **`None`이면 생략되는 series/picking provenance 키**:
  `SeriesConfig.source_id`, `PickedPointRef.source_id`, 그리고 모든
  `PickedDataRef` 변종의 `source_id`.
- **`None`이면 생략되는 outer style mapping 키 (6개)**:
  `DataScatterStyleConfig.point_style_table` / `point_style_index_column` /
  `point_style_overrides`, `DataErrorBarStyleConfig.error_bar_style_table` /
  `error_bar_style_index_column` / `error_bar_style_overrides`.
- **`None`이면 생략되는 nested style 키 (7개)**:
  `DataScatterPointStyleConfig.point_color` / `point_shape` / `point_size`,
  `DataErrorBarPointStyleConfig.error_bar_color` / `error_bar_width` /
  `error_bar_cap_size` / `cap_width`. 따라서 모든 nested style option이
  `None`이면 객체는 `{}`로 직렬화된다. override의 `style`은 flatten되므로
  이 키들은 override 객체에서도 같은 방식으로 생략된다.
- 위 `Option` 키들과 달리 `SeriesConfig.label`은 항상 존재하며 값만
  `null`일 수 있다.
- **`None`이면 생략되는 contour 키 (2개)**: `ContourConfig.per_level_color`,
  `ContourConfig.labels`. `per_level_color`가 없다는 것은 **모든 레벨이
  `line.line_color` 단색**이라는 뜻이고, 빈 배열 `[]`(항목 0개인 표)과는
  다른 진술이다.
- 세그먼트 오버라이드 키와 Config의 `draw_style` / `picked_points` / `picked_data` /
  `colorbar` 키도 생략 가능하다 (`draw_style`: `precise` = 키 자체가 생략,
  두 picked 키: `None` = 키 자체가 생략). `picked_points: {}` 와
  `picked_data: {}` 는 각각의 default overlay config로 파싱된다. 부분
  업데이트가 아니라 **전체 트리 교체**이므로, `get_config()` 결과를 고쳐서
  되돌리는 패턴을 쓸 것.

15개 omission의 canonical serde 출력은 다음과 같다. `series.source_id`, 두
outer style의 mapping 키 6개, 두 빈 nested style의 option 키 7개,
`picked_point.source_id`가 모두 생략되어 있다.

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

### Timestamp label format

Timestamp labels are configured through `LabelFormat`, not through a new axis
scale. Data coordinates remain numeric and timestamp calendar ticks are used
only on linear axes.

Default timestamp shape:

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

Use `"unit": "Milliseconds"` for JS timestamps. Use
`"timezone": { "FixedOffsetMinutes": 540 }` for KST-like fixed offsets.
Custom labels can use `"label": { "Pattern": "%Y-%m-%d %H:%M:%S.%f" }`;
supported tokens are `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%f`, and `%%`.
`AutoCalendar` measures label text and chooses coarser calendar tick units when
needed so adjacent labels do not overlap.

The exact serde forms of all 12 timestamp variants are:

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

For large absolute Unix timestamps, upload browser data with
`register_column_f64(id, Float64Array)` for a new id and
`update_register_column_f64(id, Float64Array)` for an explicit replacement, so
the renderer can preserve sub-f32 deltas as GPU `(hi: f32, lo: f32)` pairs.
Use the corresponding `register_column_f32` / `update_register_column_f32`
methods for ordinary numeric coordinates or already-relative time values.
Registration rejects an existing id; update rejects a missing id and every
accepted update performs an upload. `set_series` only selects registered column
ids and never uploads column contents.

The local demo [timestamp-demo.html](timestamp-demo.html) wires this path end to
end: `Float64Array` timestamp upload, `LabelFormat::Timestamp`, `AutoCalendar`
tick planning, chart-width changes, and export scale changes.

## 편집 시 의미 결합 주의

- `scale`을 바꾸면 `major_spacing` 해석도 바뀐다 — Linear는 데이터 단위,
  Logarithmic은 **decade 단위** (예: `1.0` = 한 자릿수마다 major 틱).
  데이터 범위가 1 decade 미만이면 decade 틱이 0개일 수 있다.
- `min` / `max`: Logarithmic에서는 양수만.
- `out_margin`을 줄이면 라벨/타이틀이 잘릴 수 있다 (레이아웃 기여 마진).
- `line_offset`은 분리 축 오프셋 — 레이아웃 비기여, 데이터 영역 불변.
- `inverted`는 축의 시각 방향을 반전한다. tick/grid, 데이터 렌더링,
  `pick_point`는 같은 반전 mapping을 사용하며 `min`/`max`는 데이터 공간
  bound로 유지된다.
- `pick_point`는 실제 scatter marker 크기(스타일 매핑 포함)와 line stroke를
  hit 대상으로 본다. line stroke 근처 클릭은 hit segment의 가까운 endpoint
  데이터 점으로 스냅되고, errorbar stem/cap 자체는 pick target이 아니다.
  `<figgy-chart>` facade 반환형은
  `Promise<{ source_id: string | null, series_id, point_index, distance_px } | null>`이다.
  raw `FiggyChart`는 같은 필드의 JSON string 또는 `undefined`를 Promise로
  반환하고 facade가 이를 object / `null`로 정규화한다.
  제출된 비동기 ticket은 제출 시점의 `source_id` / `series_id` identity를
  소유하므로, Promise가 pending인 동안 chart/pool이 변경되거나 renderer가
  해제되어도 그 요청의 identity가 다른 데이터로 바뀌지 않는다.
  좌표가 필요하면 host가 `point_index`로 자신이 등록한 원본 column을 조회한다.
- `pick_data`는 point/line에 더해 Histogram bin, Heatmap cell, Contour level을
  같은 GPU 요청으로 고른다. facade 반환값은 `kind`가
  `"point" | "histogram_bin" | "matrix_cell" | "contour_level"`인 tagged
  object 또는 `null`이다. bin은 `bin_index`, cell은 canonical axis 방향의
  `x_index`/`y_index`, contour는 `level_index`와 hit가 있던 sample-cell의
  `x_index`/`y_index`를 제공한다. `HeatmapContour`에서 선과 cell이 겹치면
  나중에 그려지는 contour가 우선하며, 시리즈 간 동률은 chart paint order를
  따른다. CPU는 bar rectangle, cell bounds, contour endpoint나 f64 값을
  복원하지 않는다. 실제 hit geometry는 보이는 draw entry와 같은 transform,
  pool, lattice, contour 함수에서 계산된다.
- `chart_area`는 저장/Export 기준의 논리 문서 사각형이다. Web wrapper의
  `resize(w, h)`는 캔버스 surface만 바꾸고, 이 논리 문서를 현재 viewport에
  uniform scale + letterbox로 맞춰 보여준다. 브라우저 창 크기 변경을
  `chart_area` 편집으로 취급하지 말 것.
- `set_series` / `apply_color_cycle` 같은 일반 시리즈 변경은 인식 가능한
  자동 범례 엔트리의 **심볼 세그먼트만 갱신**한다. `'\t'` 앞의 고정폭
  심볼 필드가 기호 영역이고, `'\t'` 뒤 텍스트는 사용자 작성 영역으로
  보존된다. 선 색뿐 아니라 `line_style` 의 dash/dot 패턴도 기호에 반영된다.
- `set_series_label(id, label)` 은 해당 엔트리 텍스트만 바꾸고, 빈 문자열은
  해당 행만 제거한다. `set_config` 로 직접 편집한 `legend.content` 는 이후
  시리즈 변경에서도 전체 재작성되지 않는다.
- 전체 범례를 `SeriesConfig.label` 기준으로 다시 만들고 싶을 때만
  `reset_legend_from_series_labels()` 를 명시 호출한다. 이때 legacy
  `add_line_series(..., label)` 로 저장된 wrapper label 은 rich label 이 없는
  시리즈의 fallback 으로만 쓰인다.

## `picked_data` — typed data selection overlay (Config 선택 키)

`Config.picked_data`는 `pick_data` 결과의 안정적인 identity만 보관한다.
`set_picked_data(json)`은 이 필드만 교체하고 JSON `null`은 overlay를 지운다.
기존 point-only `picked_points`와 독립적이므로 둘을 동시에 쓸 수 있다.

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

각 ref에는 선택적으로 `source_id`를 넣어 같은 `series_id`를 가진 서로 다른
host source를 구분할 수 있다. point는 기존 ring entry를, histogram은 선택된
instance의 동일 edge/value/style bind group을, matrix/contour는 일반 draw가
사용한 동일 field bind group을 읽는다. 따라서 축 범위나 데이터가 바뀌어도
보이는 데이터와 overlay가 서로 다른 CPU 복원 좌표를 가질 수 없다. 유효 범위를
벗어난 stale index는 그리지 않는다.

## `draw_style` — 렌더 스타일 (Config 선택 키)

`Config.draw_style`은 `DrawStyle` enum이다 (`crates/model/src/config.rs`) —
internally-tagged: `"mode"` 태그와 그 스타일의 파라미터가 **같은 객체에
인라인**된다. **키 부재 = `precise` = 정밀 모드** — 디폴트이며 현행
렌더와 동일하다. `precise`는 직렬화에서 키 자체가 생략되므로 아래 기본값
JSON 블록에도 나타나지 않는다 (`{ "mode": "precise" }` 명시도 허용).
`{ "mode": "sketch" }`를 주면 손그림(hand-drawn) 모드가 켜진다. 모드는
**차트 전역**(Config 레벨) — 시리즈별 혼합은 없다.

sketch의 모든 파라미터에 디폴트가 있어 (`serde(default)`) 부분 지정이
가능하다 — `"draw_style": { "mode": "sketch" }` 만으로 전부 디폴트로
켜진다.

| 필드 (`mode: "sketch"`) | 타입 | 디폴트 | 의미 |
|---|---|---|---|
| `amplitude_px` | f32 | `1.5` | 경로 수직 교란 진폭 (px) |
| `wavelength_px` | f32 | `60.0` | 교란 파장 (px) — 경로를 따라 이 간격마다 굴곡 1회 |
| `seed` | u32 | `0` | 전역 시드 — 같은 (config, 데이터)면 결과 픽셀 동일 |

전체 형태: `"draw_style": { "mode": "sketch", "amplitude_px": 1.5,
"wavelength_px": 60.0, "seed": 0 }` — 정밀 모드로 되돌리려면 키를
제거한다(또는 `{ "mode": "precise" }`).

`milkyway`도 같은 `draw_style` 키를 사용한다. 기존 천체사진 스타일이며,
모든 파라미터는 default가 있으므로 `"draw_style": { "mode": "milkyway" }`만으로
활성화된다.

| 필드 (`mode: "milkyway"`) | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `star_density` | f32 | `14.0` | arc 100px당 별 밀도 |
| `ribbon_width_px` | f32 | `14.0` | 시리즈색 성운 리본 폭 |
| `ribbon_intensity` | f32 | `0.30` | 리본 밝기 |
| `star_scale` | f32 | `1.0` | 별 크기 배율 |
| `star_brightness` | f32 | `1.0` | 별 광량 배율 |
| `spread_px` | f32 | `2.5` | 별 위치 산포 |
| `structure_scale` | f32 | `1.0` | 클럼핑/구조 스케일 |
| `faint_bias` | f32 | `3.0` | 어두운 별 쪽 편향 |
| `glow` | f32 | `0.55` | 축/배경 glow 강도 |
| `nebula` | f32 | `1.0` | 배경 성운 강도 |
| `dust` | f32 | `1.0` | 배경 먼지 밀도 |
| `planet_rim` | f32 | `0.34` | scatter 행성 rim 강도 |
| `seed` | u32 | `0` | 전역 시드 |

전체 형태: `"draw_style": { "mode": "milkyway", "star_density": 14.0,
"ribbon_width_px": 14.0, "ribbon_intensity": 0.30, "star_scale": 1.0,
"star_brightness": 1.0, "spread_px": 2.5, "structure_scale": 1.0,
"faint_bias": 3.0, "glow": 0.55, "nebula": 1.0, "dust": 1.0,
"planet_rim": 0.34, "seed": 0 }`.

`constellation`은 `ScatterLine` 시리즈만 지원한다. scatter 위치에는 PSF 별을
그리고, line은 기본적으로 반투명하게 연결한다.

| 필드 (`mode: "constellation"`) | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `star_opacity` | f32 | `1.0` | 별 투명도 |
| `line_opacity` | f32 | `0.45` | 연결선 투명도 |

전체 형태: `"draw_style": { "mode": "constellation", "star_opacity": 1.0,
"line_opacity": 0.45 }`.

## `get_config()` 전체 형태 — 기본값 기준

기본 `Config`는 `draw_style: Precise`, `picked_points: None`,
`picked_data: None` 이므로 `get_config()` JSON에는 세 키가 정상적으로
생략된다. overlay를 켜려면 해당 picked 객체를 추가한다. 빈 객체 `{}` 는
각 overlay의 기본 설정으로 파싱된다.

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

## `colorbar` — 컬러바와 z 스케일 (Config 선택 키)

`Config.colorbar`는 컬러바 하나이면서 **그 차트의 z 스케일 소유자**다. 기본
`Config`에는 없으므로(`None` = 키 자체가 생략) z 차원이 없는 문서 — 즉 면
계열이 생기기 전에 쓰인 모든 문서 — 는 그대로 파싱된다.

핵심은 `axis`가 4축과 **같은 `AxisOptions`** 라는 점이다. `scale`
(`"Logarithmic"` 포함) · `min`/`max` · `major_spacing` · `minor_count` ·
`label_style`(`"Power"` 포함) · `tick` · `title_option` 이 축과 완전히 같은
의미이고, 틱 생성 · 라벨 포맷 · 로그 처리가 **같은 코드**를 지난다.

| 필드 | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `visible` | bool | `true` | `false`면 아무것도 그리지 않고 **밴드도 반납**한다(면은 정상 렌더) |
| `side` | Side | `"Right"` | `Left`/`Right` = 수직 바, `Top`/`Bottom` = 수평 바. 이것만으로 방향이 결정된다 |
| `thickness_px` | f32 | `18.0` | 스트립의 짧은 쪽 |
| `gap_px` | f32 | `24.0` | 데이터 영역과 스트립 사이 |
| `length_frac` | f32 | `0.75` | 그 변 길이에 대한 스트립 길이 비율. `(0, 1]` |
| `align` | BarAlign | `"Center"` | 변을 따라 어디에 둘지 (이산 앵커) |
| `offset_x`, `offset_y` | f32 | `0.0` | 앵커에서의 자유 이동, 화면 픽셀. **마진에 기여하지 않는다**(범례·타이틀·라벨 offset과 같은 계약) — 드래그가 여기에 누적되므로 바를 끌어도 데이터 영역이 다시 흐르지 않는다 |
| `colormap` | ColorMap | `"Viridis"` | 연속 램프 |
| `nan_color` | Color | 완전투명 | 램프에 놓을 수 없는 z(NaN, 로그 컬러바의 비양수) |
| `border_color` | Color | 회색(80,80,80) | 스트립 테두리 |
| `border_width` | f32 | `1.0` | 〃 |
| `axis` | AxisOptions | 아래 | **z 축. z 범위의 진실 원본** |

따라오는 규칙:

- `Heatmap` / `Contour` / `HeatmapContour` 시리즈가 있으면 이 키가 **있어야
  한다.** 없으면 z 범위도 colormap도 어디에도 없어 그릴 값 자체가 없으므로
  시리즈가 거부된다.
- 결과적으로 **차트당 z 스케일 1개**다. 한 차트의 히트맵 여러 개는 같은
  스케일을 공유한다.
- 컬러바 밴드 = `gap_px + thickness_px + axis.out_margin +
  axis.major_tick_length` 이고, 그 변의 마진에 더해져 데이터 영역이 줄어든다.
  `fit`/`resize`는 `axis.out_margin`(라벨 공간)만 조절하고 스트립 자체
  (`thickness_px`/`gap_px`/틱 길이)는 건드리지 않는다.
- 컬러바 축의 기본값은 4축과 두 곳이 다르다: `line_visible: false`(스트립
  테두리가 그 선 역할을 한다)와 `tick: "Outside"`(틱이 색 위가 아니라 라벨
  마진에 놓인다). `tick`은 `None`/`Outside`/`Inside`/`Both` 방향을,
  `inverted`는 min→max 화면 방향을 정하며, 틱 외형은 축선과 같은
  `line_color`/`line_width`/`line_style`을 쓴다.
- 컬러바는 **선택 · 이동 · 크기조정**이 되는 요소다. 히트테스트 id는
  `"colorbar"`이고, 선택하면 파란 박스 + **8개 크기조정 핸들**이 붙는다(데이터
  영역과 함께 핸들을 가진 둘뿐인 요소). 이동은 `offset_{x,y}`에, 크기조정은
  핸들이 잡은 변에 따라 `thickness_px`(짧은 쪽) 또는 `length_frac`(긴 쪽)에
  들어간다 — 어느 쪽인지는 **바의 방향**이 정하고 핸들은 화면 방향만 안다.
- 틱/축, 틱 라벨, 제목은 `"colorbar_axis"`, `"colorbar_tick_labels"`,
  `"colorbar_title"`로 따로 선택되고 파란 선택 표시가 붙는다. 드래그는 각각
  `axis.line_offset`, `axis.label_style.label_offset_{x,y}`,
  `axis.title_option.offset_{x,y}`를 갱신한다. 배치와 히트박스는 모두 실제
  컬러바 스트립 사각형에서 파생되므로 `length_frac`/`align`/bar offset/resize와
  정확히 함께 움직인다.
- 웹 편집은 `set_colorbar_axis(json)`으로 이 `AxisOptions` 전체를 교체하거나,
  `set_colorbar_title(text)`로 제목을 바로 설정할 수 있다. 후자는 빈 문자열이면
  제목을 숨기며, 두 호출 모두 컬러바가 없으면 실패한다.

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

## 면 · 막대 render_type — 전체 형태

새 4종의 canonical 형태다. `Histogram`은 호스트가 미리 비닝한
`(edges, counts)` 두 컬럼을 받고, 나머지 셋은 `MatrixRef`로 격자를 선언한다.

- `Histogram`: `bar.orientation`이 컬럼 역할을 **단독 결정**한다.
  `"Vertical"` = `x_column`이 edges / `y_column`이 counts, `"Horizontal"`은
  반대. 길이 관계(`edges = counts + 1`)로 역할을 추측하지 않는다.
- `bar.width_ratio`는 각 bin 폭에서 가운데 정렬된 막대가 차지할 비율이며
  `0..=1`로 clamp된다. 그 다음 `gap_px`가 양쪽에서 픽셀 단위로 추가 차감된다.
  양수 폭 막대가 1픽셀보다 넓으면 최소 1픽셀을 남기도록 gap이 제한되고, 이미
  원래 bin 폭이 1픽셀 미만이면 픽셀 열별 최댓값을 GPU에서 골라 0까지 채운다
  (가로 히스토그램은 픽셀 행). 이 경로에서는 gap과 양수 width_ratio를 무시하며,
  최댓값 bin의 선 두께와 알파가 양수면 영역 전체를 선 색으로 채우고,
  아니면 면 색으로 채운다. 동률은 앞선 bin을 선택하고,
  `width_ratio: 0`인 bin은 제외한다. 원본 컬럼은 변경하지 않는다.
  `border_width: 0`은 외곽선을 끄며 `border_color`와 양수 두께가 외곽선을 정한다.
- `bar.bar_style_overrides`는 `index`로 특정 bin 하나를 골라 `fill_color`,
  `border_color`, `border_width`, `gap_px`, `width_ratio` 중 필요한 값만 덮는다.
  같은 index가 여러 번 나오면 선언 순서대로 합성된다. baseline과 orientation은
  시리즈 단위다. 렌더·typed pick·선택 outline이 모두 이 최종 막대 경계를 쓴다.
- `MatrixRef.columns`: 격자를 이루는 구성 컬럼 id 목록. 별도의 데이터 보유
  객체는 없다 — 격자는 **풀에 있는 그 컬럼들 자체**다.
- `MatrixRef.grid_layout`: 좌표 컬럼이 셀 경계(`"Edges"`, n+1개)인지 셀
  중심(`"Centers"`, n개)인지. **추론하지 않는다.**
- 선언과 데이터의 개수가 어긋나도 **에러가 아니다.** 가장 작은 공통 범위까지
  그리고 잘렸다는 사실만 알린다.
<!-- contour-contract: scope=schema max-levels=1024 -->
- `ContourConfig.levels`는 항상 **데이터 단위**의 명시 목록이다(자동 추론
  variant 없음). 허용 길이는 `0..=1024`이고 1025개 이상이면 `set_series`가
  실패한다. 배열을 조용히 자르지 않으며 이전 config, series, GPU style은
  그대로 유지된다.
- `ContourLabelConfig.spacing_px`는 자동 배치의 목표 간격이며 숨김 라벨과 명시
  anchor에서도 유한한 양수여야 한다. 정상 선택은 이 간격을 목표로 하지만
  레벨별 fallback은 더 가까운 후보를 남길 수 있다. automatic/explicit은 공통
  1024개 용량을 쓴다. 명시 `anchors`는 유효하지 않은 `level_index`를 제거한 뒤
  입력 순서의 앞 1024개만 사용한다. resolved 목록이 비면 자동 배치하고,
  하나라도 남으면 그 목록이 자동 배치를 대체한다. automatic에서만 clamp된
  frame/export scale을 spacing에 곱해 유한한 양수인지 다시 검사한다. atlas는
  WebGPU adapter의 texture dimension 안에 들어야 하며, 위 검증 실패는 이전
  config, series, GPU style을 보존한다. 앵커는 데이터 좌표 `(x, y)` + 데이터
  공간 접선 `(tx, ty)`로 저장되므로 줌/팬 때 각도만 다시 투영하면 된다.
- `ContourLabelConfig.color`는 contour 선색/`per_level_color`와 독립적인 글자색이다.
  Decimal은 level 간격(없으면 colorbar 간격)과 `significant_digits`를 함께 사용한다.
  선은 실제 선택된 label 사각형 안에서 끊기며 `bg_padding_px`는 배경색 유무와 무관하게
  그 간격을 패딩한다.

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

## `get_series()` 전체 형태 — 최대 변형 예시

`LineScatterErrorbarXY` + 두 가지 `ErrorRef` 형태 + 라벨이 모두 포함된
한 개짜리 배열. 실제 값은 이 형태의 부분집합 변형들이다.

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

Errorbar style mapping is independent from scatter style mapping. `error_bar_style_index_column`
selects rows from `error_bar_style_table`, and `error_bar_style_overrides` applies sparse
per-point exceptions by source point index. Styled draw modes ignore this mapping; the precise
errorbar path applies it to color, stem width, cap half-size, and cap width only.
