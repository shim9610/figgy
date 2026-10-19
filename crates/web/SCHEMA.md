<a id="config--seriesconfig--json-스키마-레퍼런스"></a>

# Config / SeriesConfig JSON 설정 명세

`crates/web`의 [브라우저 통합 예제](README.md)가 사용하는 JSON 형식이다.
설정 타입 자체는 Rust 모델에 정의되며 `serde`로 직렬화할 수 있다.

이 문서는 `figgy` 0.15.0과 `figgy-renderer` 0.18.0 / `figgy-model` 0.12.0의 JSON 형식을 설명한다. `crates/web`은 같은 체크아웃의 렌더러를 경로 의존성으로 빌드한다. Rust 호스트에서 같은 JSON을 읽고 쓰려면 렌더러의 `serde` 기능을 켠다.

```toml
renderer = { package = "figgy-renderer", version = "0.18.0", features = ["serde"] }
```

figgy-model 0.11.0과 0.10.0이 쓴 문서는 그대로 읽히며 다시 직렬화해도 같은 바이트가 된다. 반대로 `polar`, `overlays`, `subplots`, `active_layer`, `annotations`, `point_color_map` / `point_size_map`, 선 스타일의 `fill` / `fill_color` / `interpolation`, `RichSegment.rule_band` 같은 키가 든 문서를 figgy-model 0.11.0 이하에서 읽으면 이 키는 오류 없이 무시되고, `"Vector"`나 `"Degrees"` 같은 열거형 값은 거부된다.

GPU 컬럼 풀에 올리지 않는 비상주(스트리밍) 컬럼 등록, 원본 구간 공급, `render_chart()` 실행·취소, 컬럼을 풀에 올릴 수 있는지(상주 가능 여부) 조회는 설정 필드가 아니라 별도 웹 API로 다룬다. 지원 범위와 호출 방법은 [WASM.md](../renderer/WASM.md#exact-streaming)의 「스트리밍 자동 실행과 원본 구간 공급」을 참고한다.

아래는 `FiggyChart.get_config()` / `get_series()`가 반환하고 `set_config()` / `set_series()`가 받는 JSON의 전체 형식이다. 예시 JSON은 소스와 자동으로 대조하지 않으므로, 정확한 형식은 `get_config()` / `get_series()`의 실제 출력으로 확인한다.

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
- **데이터를 담은 열거형 대부분**은 종류 이름을 바깥쪽 키로 쓴다(externally tagged). 예: `"render_type": { "Line": { "line": { … } } }`, `"err_x": { "Asymmetric": { "lower": "…", "upper": "…" } }`.
- **선택 참조는 내부 태그를 사용한다.** `PickedDataRef`는 `{ "kind": "histogram_bin", "series_id": "hist", "bin_index": 2 }`처럼 `kind`와 필드를 같은 객체에 둔다. `{ "HistogramBin": { ... } }` 형식이 아니다. `DrawStyle`은 같은 방식으로 `mode`를 태그로 쓴다.
- **값 하나를 감싼 타입(newtype)**은 내부 값을 그대로 직렬화한다. 예: `ChartArea(Rect)` → `"chart_area": { "x": …, "width": … }`.
- **`RichSegment.text`는 문자 하나**다. JSON에서는 `"V"` 같은 한 글자 문자열로 표시한다.
- **텍스트 구간별 색·크기는 선택 항목**이다. `RichSegment.color` / `font_size`는 별도로 지정했을 때만 키를 내보낸다. 없으면 `RichText.color` / `font_size`를 따른다. 범례 기호의 색도 구간의 `color`로 지정한다. 반면 `bold`, `italic`, `underline`, `superscript`, `subscript`, `greek`는 생략할 수 없다. 완전한 구간은 `{"text":"●","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":false,"color":{...}}` 형태이며, 아래 기호 예시는 이 여섯 키를 줄여서 보인다.
- **`"\t"` 구간은 열 구분자**다. 각 열의 너비를 문서 전체에서 가장 넓은 셀에 맞추며 탭 자체는 그리지 않는다.
- **`RichText.align`은 여러 줄 텍스트의 줄 정렬**이다. `"Left"`(기본), `"Center"`, `"Right"` 중 하나이며 기본값이면 키를 내보내지 않는다. 블록 너비는 가장 넓은 줄이 정하므로 정렬해도 측정 크기와 선택 영역은 바뀌지 않는다.
- **`rule_band`는 채움이 있는 선의 범례 띠**다. `rule: true`인 구간에 `rule_band: Color`가 있으면 수평선 아래에 그 색의 띠를 같은 고정 너비 안에 그린다. 생략하면 선만 그린다. 자동 범례(`set_series`, `apply_color_cycle`)는 `fill`이 있는 선 시리즈의 기호에 채움색을 띠 색으로 넣는다.
- **`field_em`은 기호 영역의 너비를 고정한다.** 다음 글자까지의 간격을 `field_em × 폰트 크기`로 잡고 글리프를 가운데 놓는다. `rule: true`이면 글리프 대신 그 영역을 채우는 수평선을 그린다. `rule_dash`는 선·공백 길이를 em 단위로 지정하므로 폰트 크기에 비례한다. 범례 기호는 모양에 관계없이 전체 너비가 2.0em이다. 선은 `{"text":"—","rule":true,"field_em":2.0,"color":{...}}`, 점은 `{"text":"●","field_em":2.0,...}`, 점선은 `{"text":"—","rule":true,"field_em":2.0,"rule_dash":[0.571,0.286],...}`로 표현한다. 선과 점의 조합은 선 0.65em + 글리프 0.7em + 선 0.65em으로 구성한다.
- **색은 0~1 범위의 RGBA 실수 값**이다. 예: `{ "r": 0.8, "g": 0.1, "b": 0.1, "a": 1.0 }`.
- **데이터 출처 키**인 `SeriesConfig.source_id`, `PickedPointRef.source_id`, 모든 `PickedDataRef`의 `source_id`는 `None`이면 생략한다.
- **시리즈의 스타일 매핑 키 6개**도 `None`이면 생략한다. `DataScatterStyleConfig`의 `point_style_table` / `point_style_index_column` / `point_style_overrides`와 `DataErrorBarStyleConfig`의 `error_bar_style_table` / `error_bar_style_index_column` / `error_bar_style_overrides`가 해당한다.
- **연속 값 매핑 키 2개**인 `DataScatterStyleConfig.point_color_map` / `point_size_map`도 `None`이면 생략한다. `point_color_map`은 `{ "column": "<id>" }`이며 색은 `Config.colorbar`가 정하므로 `colorbar`가 있어야 한다. `point_size_map`은 `{ "column": "<id>", "domain_min": 0.0, "domain_max": 10.0, "min_px": 2.0, "max_px": 12.0, "scale": "Area" }`이다. `scale`은 `"Area"`(기본값) 또는 `"Radius"`이며, 입력에서는 생략할 수 있지만 출력에는 항상 들어 있다. 두 컬럼 모두 등록돼 있어야 하고 시리즈의 x/y 컬럼과 인덱스가 맞아야 한다. 자세한 내용은 [점 색·크기 매핑](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#점-색과-크기-매핑)을 참고한다.
- **벡터 시리즈**의 `render_type`은 `{ "Vector": { "components": ..., "style": ... } }`이다. `components`는 `{ "Points": { "u": "<id>", "v": "<id>" } }` 또는 `{ "Grid": { "u": MatrixRef, "v": MatrixRef } }`이며, 격자 형식의 두 행렬은 `orientation`과 `grid_layout`이 같아야 한다. `style`(`VectorStyle`)의 필수 키는 `color`, `width_px`, `scale`, `head_length_px`, `head_width_px`다. `scale`은 `{ "Pixels": { "px_per_unit": 1.0 } }` 또는 `{ "Data": { "factor": 0.5 } }`다. `pivot`은 `"Tail"`(기본값)·`"Middle"`·`"Tip"` 중 하나이며, 입력에서는 생략할 수 있지만 출력에는 항상 들어 있다. `min_length_px`는 `0.0`이면, `color_map`은 `None`이면 생략한다. `color_map`은 점 형식에서 `{ "Column": "<id>" }`, 격자 형식에서 `{ "Grid": { "columns": [...], "orientation": "...", "grid_layout": "..." } }`이며 `Config.colorbar`가 있어야 한다. 전체 예는 [`Vector` render_type의 전체 형식](#vector-render_type의-전체-형식), 필드 의미는 [벡터장](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#벡터장)을 참고한다.
- **개별 데이터의 스타일 키 7개**도 `None`이면 생략한다. `DataScatterPointStyleConfig`의 `point_color` / `point_shape` / `point_size`와 `DataErrorBarPointStyleConfig`의 `error_bar_color` / `error_bar_width` / `error_bar_cap_size` / `cap_width`가 해당한다. 모두 비어 있으면 `{}`로 직렬화한다. 개별 덮어쓰기 항목의 `style`은 한 단계 풀어서 저장하므로 같은 생략 규칙을 따른다.
- `SeriesConfig.label`은 위 선택 키들과 달리 항상 존재하며 값으로 `null`을 가질 수 있다.
- **선 스타일의 선택 키 3개**인 `DataLineStyleConfig.fill` / `fill_color` / `interpolation`은 기본값이면 생략한다. `fill`은 `{ "ToBaseline": 0.0 }` 또는 `{ "ToSeries": "<series_id>" }`이고, `fill_color`는 `Color`(생략하면 `line_color`의 알파를 0.25배 한 색), `interpolation`은 `"Linear"`(기본)·`"StepPre"`·`"StepPost"`·`"StepMid"`다. `ToSeries`는 같은 차트에서 같은 `x_column`을 쓰는 다른 시리즈를 가리켜야 하며, 두 시리즈 모두 상주 컬럼이어야 한다. 자세한 의미는 [면 채움과 계단선](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#면-채움과-계단선)을 참고한다.
- **등고선의 선택 키 2개**인 `ContourConfig.per_level_color` / `labels`는 `None`이면 생략한다. `per_level_color`를 생략하면 모든 레벨을 `line.line_color`로 그린다. 목록이 레벨 수보다 짧으면 색이 없는 레벨도 `line.line_color`로 그리므로, 빈 목록 `[]`도 생략한 것과 같게 그려지지만 JSON에는 `[]`로 남는다.
- 텍스트 구간의 선택 키와 `Config`의 `draw_style` / `picked_points` / `picked_data` / `colorbar`도 생략할 수 있다. `draw_style`은 기본 정밀 모드일 때, `picked_points` / `picked_data` / `colorbar`는 `None`일 때 생략한다. `picked_points: {}`와 `picked_data: {}`는 각각 기본 선택 표시 설정으로 해석한다.
- **극좌표 키** `polar`는 `None`이면 생략한다. `{ "polar": { "zero_angle_degrees": 0.0, "clockwise": false, "angle_unit": "Degrees", "sector": null, "inner_radius_fraction": 0.0, "interpolation": "Chord", "radial_label_angle_degrees": 90.0 } }`가 전체 형태이며 모든 필드에 기본값이 있어 `{ "polar": {} }`만으로 극좌표를 켠다. `angle_unit`은 `"Degrees"` / `"Radians"`, `sector`는 `[start, end]` 또는 `null`, `interpolation`은 `"Chord"` 또는 `{ "Arc": { "pieces": 8 } }`다. 켜면 `bottom_x`가 각도 축, `left_y`가 반지름 축이고 산점도·선(채움 포함)만 그린다. 각도는 `x_column` 값을 `angle_unit` 단위의 각도로 보고 `zero_angle_degrees`에서부터 재므로 원은 항상 한 바퀴(360° 또는 2π)다. `bottom_x`의 `min` / `max`는 각도 매핑을 바꾸지 않고 그릴 각도 눈금·스포크·라벨만 정하며, 원의 일부만 그리려면 `sector`를 쓴다. 히스토그램·행렬·오차 막대·벡터 시리즈, `overlays`, 스트리밍 소스는 오류로 거부한다. 필드 의미와 검증 규칙은 [설정 레퍼런스](../renderer/CONFIG.md#polaroptions-configpolar-1)에 있다.
- **레이어 키 4개**인 `overlays`, `subplots`, `active_layer`, `annotations`도 기본값이면 생략한다. bounded 차트는 `FiggyBoundedChart`가 그리는 `radial`·`categorical`·`boxplot`·`radar` 문서로, 컬럼 대신 문서 안에 값을 담는다. bounded 차트 문서에는 네 레이어 키 가운데 `active_layer`와 `annotations`만 있고 `overlays`와 `subplots`는 없다. `active_layer`와 `annotations`의 필드 의미·기본값·검증 규칙은 [설정 레퍼런스](../renderer/CONFIG.md#layerref--annotationlayer-1)에 있다.
- **활성 레이어 키** `active_layer`는 `"Base"`(기본), `{ "Overlay": 0 }`, `"Annotations"`, `{ "Subplot": 0 }`, `{ "SubplotOverlay": [0, 0] }` 중 하나다. 데이터와 장식의 그림은 바꾸지 않고 히트 맵 등록과 선택 표시, `auto_fit_all`이 맞출 축에만 영향을 준다. 범위 밖의 `Overlay`·`Subplot` 인덱스와 서브플롯 인덱스가 범위 밖인 `SubplotOverlay`는 `"Base"`로, 서브플롯은 있지만 그 오버레이가 없는 `SubplotOverlay`는 해당 `{ "Subplot": i }`로 저장한다. `set_active_layer(json)` / `active_layer()`가 이 키만 바꾸고 읽는다. 저수준 `FiggyChart`는 저장된 값을 JSON 문자열로, `<figgy-chart>`는 파싱한 값으로 돌려준다. `auto_fit_all(pad)`는 활성 레이어의 축만 맞춘다. `"Base"`와 `"Annotations"`는 부모의 `bottom_x`·`left_y`를, `{ "Subplot": i }`는 서브플롯 `i`의 `bottom_x`·`left_y`를, `{ "Overlay": n }`과 `{ "SubplotOverlay": [i, n] }`은 그 오버레이의 `y_axis`를 맞춘다. 각 축은 그 축에 그려지는 시리즈로 맞추므로 오버레이 시리즈는 자기 차트의 X 축에는 들어가고 `left_y`에는 들어가지 않는다. 링크된 축(`link_x`, `link_y`)은 X와 Y를 따로 따져 링크 묶음 전체의 시리즈로 맞추고 묶음 전체를 움직인다. 자세한 규칙은 [WASM 가이드의 범위 맞춤](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md#5-빌드-산출물cratesweb--pkg)을 참고한다. `"Annotations"`이면 포인터 처리(`hit_test`, `on_press`, bounded 차트의 `hit_decoration`)가 주석 항목을 가장 먼저 검사해 `annotation_<n>`을 반환하고, 다른 값이면 클릭이 주석을 통과한다. bounded 차트에는 오버레이와 서브플롯이 없으므로 `Overlay`·`Subplot`·`SubplotOverlay` 값을 `"Base"`로 읽고 저장하며, 값이 바뀌면 `decorations.selected`를 지운다.
- **주석 레이어 키** `annotations`는 `{ "visible": true, "items": [...] }`이며 빈 `items`는 생략한다. 각 항목은 `{ "id": "mean", "visible": true, "kind": { "ReferenceLine": { "axis": "Y", "value": 0.5, "style": { "color": {...}, "width": 1.5, "style": "Dash" }, "label": <RichText | null> } } }` 형태이고 `id`가 빈 문자열이거나 `visible`이 `true`면 그 키를 생략한다. `kind`는 `ReferenceLine` / `Span { axis, from, to, fill }` / `Text { anchor, text, box_style }` / `Arrow { from, to, style, text }` 중 하나이며, 앵커는 `{ "Data": { "x": 1.0, "y": 2.0 } }` 또는 `{ "Ratio": { "x": 0.5, "y": 0.5 } }`다. 값이나 앵커를 배치할 수 없는 항목은 오류 없이 건너뛰고, 저수준 `FiggyChart.annotation_warnings()`(bounded 차트는 `FiggyBoundedChart.annotation_warnings()`)가 `[{ "index": 0, "reason": "OutOfRange" }]` 형식의 JSON 문자열로 알린다. `reason`은 `NonFinite`(NaN·무한대), `NoValueAxis`(값 축이 없는 원형 차트나 극좌표 차트의 `Data` 앵커·축 값), `OutOfRange`(축 범위나 패널 밖, 로그축의 0 이하 값), `EmptyText`(빈 텍스트) 중 하나다. `FiggyChart`의 `annotation_warnings()`와 주석 조회 메서드(`annotations()`, `hit_annotation` 등)는 마지막으로 그린 프레임을 기준으로 하므로, `add_annotation` / `set_annotation` / `remove_annotation`으로 바꾼 항목은 다음 `frame()`이 그린 뒤에 반영된다. `FiggyBoundedChart`는 편집 호출 안에서 바로 반영한다. 자세한 내용은 [주석 레이어 메서드](../renderer/WASM.md#annotation-layer)를 참고한다.
- **오버레이 키** `overlays`는 `OverlayLayer` 배열이며 비어 있으면 생략한다. 각 항목은 `{ "y_axis": AxisOptions, "series": ["id", ...], "legend": Legend }` 형태로, `series`는 비어 있으면, `legend`는 `null`이면 생략하고 `y_axis`는 항상 쓴다(빠지면 기본 Y 축 옵션으로 읽는다). 한 시리즈 id는 최대 한 오버레이에만 적을 수 있고, 적지 않은 시리즈는 기본 레이어에 속한다. 각 오버레이의 `y_axis`가 그 오버레이에 적은 시리즈의 Y 범위·스케일·방향을 정하고, X 축과 데이터 영역은 부모와 함께 쓴다. 스트리밍 소스 시리즈는 오버레이에 넣을 수 없다. 주석은 부모 축 기준으로 기본 레이어에 한 번 그린다.
- **서브플롯 키** `subplots`는 같은 문서에 그리는 차트 배열이며 비어 있으면 생략한다. 각 항목은 `{ "config": Config, "series": ["id", ...], "link_x": "Parent", "link_y": { "Subplot": 0 } }` 형태로, `config`는 자기 `chart_area`(부모와 같은 표면 좌표)와 축 4개, 제목, 범례, `overlays`를 가진 완전한 차트다(그 안의 `subplots`는 비어 있어야 하고 `annotations`는 기본값이어야 한다). `series`는 비어 있으면, `link_x` / `link_y`는 `null`이면 생략한다. 링크가 없으면 축은 그 차트 자신의 것이고, 링크가 있으면 `bottom_x` / `left_y`의 범위·스케일·방향·눈금 간격이 대상(`"Parent"` 또는 더 앞 번호의 `{ "Subplot": j }`)을 따른다. 한 시리즈 id는 서브플롯 하나에만 적을 수 있고, 부모의 오버레이에 적은 id는 서브플롯에 적을 수 없다. 스트리밍 소스 시리즈는 서브플롯에 넣을 수 없다. 문서 출력과 캔버스는 모든 차트 영역의 합집합을 덮는다.

다음 직렬화 예에서는 선택 키 20개를 생략했다. `series.source_id`, 선 스타일의 선택 키 3개(`fill` / `fill_color` / `interpolation`), 스타일 매핑 키 6개, 연속 값 매핑 키 2개(`point_color_map` / `point_size_map`), 개별 스타일 키 7개, `picked_point.source_id`가 해당한다.

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
| `format` (LabelFormat) | `"Decimal"` `"Scientific"` `"Power"` `{ "Timestamp": { ... } }` `"Degrees"` `"Radians"` |
| `unit` (TimestampUnit) | `"Seconds"` `"Milliseconds"` `"Microseconds"` `"Nanoseconds"` |
| `timezone` (TimestampZone) | `"Utc"` `{ "FixedOffsetMinutes": 540 }` |
| `label` (TimestampLabelMode) | `"Auto"` `{ "Pattern": "%Y-%m-%d %H:%M:%S.%f" }` |
| `fractional` (FractionalSecondDigits) | `"Auto"` `{ "Fixed": 3 }` |
| `tick_policy` (TimestampTickPolicy) | `"AutoCalendar"` `"NumericSpacing"` |
| `interpolation` (LineInterpolation) | `"Linear"` `"StepPre"` `"StepPost"` `"StepMid"` |
| `fill` (LineFill, 태그) | `"ToBaseline"` (`f64`) / `"ToSeries"` (`series_id` 문자열) |
| `line_style` (LineStylePreset) | `"Solid"` `"Dash"` `"Dot"` `"DashDot"` `"DashDotDot"` `"ShortDash"` `"ShortDot"` `"ShortDashDot"` `"LongDash"` `"LongDashDot"` `"LongDashDotDot"` |
| `corner` (LegendCorner) | `"TopLeft"` `"TopRight"` `"BottomLeft"` `"BottomRight"` |
| `point_shape` (ScatterShape) | `"Circle"` `"Square"` `"Triangle"` `"Diamond"` `"Cross"` `"CircleFilled"` `"SquareFilled"` `"TriangleFilled"` `"DiamondFilled"` `"TriangleDown"` `"TriangleLeft"` `"TriangleRight"` `"Plus"` `"Pentagon"` `"Hexagon"` `"Octagon"` `"Star"` `"TriangleDownFilled"` `"TriangleLeftFilled"` `"TriangleRightFilled"` `"PlusFilled"` `"CrossFilled"` `"PentagonFilled"` `"HexagonFilled"` `"OctagonFilled"` `"StarFilled"` |
| `render_type` (DataRenderType, 태그) | `"Scatter"` `"Line"` `"ScatterLine"` `"ScatterErrorbarX"` `"ScatterErrorbarY"` `"ScatterErrorbarXY"` `"LineScatterErrorbarX"` `"LineScatterErrorbarY"` `"LineScatterErrorbarXY"` `"Histogram"` `"Heatmap"` `"Contour"` `"HeatmapContour"` `"Vector"` |
| `err_x` / `err_y` (ErrorRef, 태그) | `"Symmetric"` (`{column}`) / `"Asymmetric"` (`{lower, upper}`) |
| `side` (Side) | `"Top"` `"Bottom"` `"Left"` `"Right"` |
| `align` (BarAlign, 색상 막대) | `"Start"` `"Center"` `"End"` |
| `align` (TextAlign, RichText) | `"Left"` `"Center"` `"Right"` |
| `colormap` (ColorMap) | `"Viridis"` `"Magma"` `"Turbo"` `"GrayScale"` `"RdBu"` `{ "Custom": { "stops": [Color, …] } }` |
| `orientation` (BarOrientation) | `"Vertical"` `"Horizontal"` |
| `orientation` (MatrixOrientation) | `"ColumnsAreX"` `"ColumnsAreY"` |
| `grid_layout` (GridLayout) | `"Edges"` `"Centers"` |
| `mode` (FillMode) | `"Continuous"` `"Bands"` |
| `shading` (Shading) | `"Flat"` `"Interpolated"` |
| `mode` (DrawStyle, 내부 태그) | `"precise"` `"sketch"` `"milkyway"` `"constellation"` |
| `scale` (SizeScale, 점 크기 매핑) | `"Area"` `"Radius"` |
| `scale` (VectorScale, 태그) | `{ "Pixels": { "px_per_unit": … } }` `{ "Data": { "factor": … } }` |
| `pivot` (VectorPivot) | `"Tail"` `"Middle"` `"Tip"` |
| `angle_unit` (AngleUnit) | `"Degrees"` `"Radians"` |
| `interpolation` (PolarInterpolation, 극좌표) | `"Chord"` `{ "Arc": { "pieces": 8 } }` |
| `active_layer` (LayerRef) | `"Base"` `{ "Overlay": 0 }` `"Annotations"` `{ "Subplot": 0 }` `{ "SubplotOverlay": [0, 0] }` |
| `link_x` / `link_y` (CellRef) | `"Parent"` `{ "Subplot": 0 }` |
| `axis` (AnnotationAxis, 주석) | `"X"` `"Y"` |

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

시간 관련 열거형 5개의 값 12가지는 다음과 같이 직렬화한다.

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

정확하게 편집하려면 `get_config()` 결과에서 바꿀 필드만 고친 뒤 전체 설정을 `set_config()`로 전달한다. `set_config()`는 일부 필드가 아니라 전체 설정을 교체한다. 예를 들어 축의 `min` / `max`만 바꾸면 현재 눈금 간격, 보조 눈금 수, 라벨 형식은 바뀌지 않는다. 축은 서로 독립이므로 짝을 이루는 두 축은 같은 범위를 써야 할 때만 함께 고친다. `auto_fit_x`, `auto_fit_all`, `set_title` 같은 편의 함수는 눈금이나 제목 배치도 함께 조정할 수 있다. `auto_fit_all`은 활성 레이어(`active_layer`)의 축만 맞춘다. 자세한 규칙은 [설정 직접 편집과 편의 함수](https://github.com/shim9610/figgy/blob/master/crates/renderer/API.md#direct-ssot-edits-and-convenience-helpers)와 [WASM 편집 흐름](https://github.com/shim9610/figgy/blob/master/crates/renderer/WASM.md)을 참고한다.

- `scale`을 바꾸면 `major_spacing`의 단위도 달라진다. `Linear`는 데이터 단위이고 `Logarithmic`은 10배 간격인 decade 단위다. 예를 들어 `1.0`은 값이 10배 커질 때마다 주 눈금을 놓는다. 범위가 10배 간격보다 좁으면 주 눈금이 없을 수도 있다.
- 로그축의 `min` / `max`에는 양수를 지정한다.
- `out_margin`은 배치에 필요한 여백이다. 너무 줄이면 라벨이나 제목이 잘릴 수 있다.
- `line_offset`은 데이터 영역을 유지한 채 축만 이동시킨다. 전체 배치에는 영향을 주지 않는다.
- `inverted`는 화면상의 축 방향을 뒤집는다. 눈금·격자·데이터·피킹은 같은 변환을 적용하며 `min` / `max`는 데이터 좌표 기준을 유지한다.
- `pick_point`는 스타일 매핑을 반영한 점 기호 크기와 선 두께를 기준으로 선택한다. 선 근처를 클릭하면 해당 선분의 가까운 끝점을 반환한다. 계단선(`interpolation`이 `Step*`)은 그려진 계단 조각을 기준으로 판정하며 경로상 위치가 가까운 끝점을 돌려준다. 면 채움(`fill`)은 선택 대상이 아니다. 오차 막대의 몸통과 끝선 자체는 선택 대상이 아니다. 웹 래퍼의 반환형은 `Promise<{ source_id: string | null, series_id, point_index, distance_px } | null>`이다. 저수준 `FiggyChart`는 같은 필드의 JSON 문자열 또는 `undefined`를 Promise로 반환하고 래퍼가 이를 객체 또는 `null`로 바꾼다. 요청은 제출 당시 식별자를 보관하므로 대기 중 차트·풀이 바뀌거나 렌더러가 해제돼도 다른 데이터를 가리키지 않는다. 좌표가 필요하면 `point_index`로 호스트의 원본 컬럼을 조회하거나, `pick_data` 결과를 `set_picked_data`로 `picked_data.refs`의 첫 항목에 넣은 뒤 `FiggyChart.selection_details()`로 읽는다.
- `pick_data`는 점·선 외에 히스토그램 구간·히트맵 셀·등고선 레벨을 같은 GPU 요청으로 선택한다. 래퍼는 `kind`가 `"point" | "histogram_bin" | "matrix_cell" | "contour_level"`인 객체 또는 `null`을 반환한다. 구간은 `bin_index`, 셀은 X·Y축 기준 `x_index` / `y_index`, 등고선은 `level_index`와 선택 지점이 속한 표본 셀의 `x_index` / `y_index`를 제공한다. `HeatmapContour`에서 선과 셀이 겹치면 나중에 그리는 등고선이 우선하며 시리즈 간 동률은 그리기 순서로 정한다. 판정은 그리기와 같은 GPU 변환을 사용하므로 화면에 그려진 위치와 일치한다. 결과에는 `source_id`(없으면 `null`)와 `distance_px`도 들어 있다.
- `chart_area`는 저장과 출력에 사용하는 문서 영역이다. 서브플롯이 있으면 문서 영역은 `chart_area`와 모든 서브플롯 `config.chart_area`의 합집합(Rust의 `Config::document_area()`)이며, `export_png`는 이 영역을 그린다. 웹의 `resize(w, h)`는 캔버스만 바꾸며 문서 영역을 같은 가로세로 비율로 맞추고 남는 공간에 여백을 둔다. 브라우저 창 크기 변경은 `chart_area` 편집과 구분한다.
- `set_series` / `apply_color_cycle`은 자동 범례에서 인식할 수 있는 기호 부분만 갱신한다. `\t` 앞의 고정 너비 영역이 기호이며, 뒤의 사용자 텍스트는 보존한다. 선 색뿐 아니라 `line_style`의 점선·도트 패턴과 `fill`의 채움색 띠(`rule_band`)도 반영한다.
- `set_series_label(id, label)`은 해당 범례의 텍스트만 바꾼다. 빈 문자열이면 해당 행을 제거한다. `set_config`로 직접 편집한 `legend.content`는 이후 시리즈 변경으로 전체를 다시 쓰지 않는다.
- 전체 범례를 `SeriesConfig.label`에서 다시 만들 때는 `reset_legend_from_series_labels()`를 호출한다. 기존 `add_line_series(..., label)`로 저장한 라벨은 리치 텍스트 라벨이 없는 시리즈에서만 대체값으로 사용한다.

<a id="picked_data--typed-data-selection-overlay-config-선택-키"></a>

## 데이터 종류별 선택 표시(`picked_data`)

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

각 참조에 `source_id`를 넣으면 같은 `series_id`를 가진 서로 다른 데이터 출처를 구분할 수 있다. 모든 강조는 `highlight_color`로 그린다. 점은 `picked_points`와 같은 고리를 `outline_width_px` 두께와 `point_radius_extra_px`만큼 커진 반지름으로, 히스토그램 막대와 행렬 셀은 `outline_width_px` 두께의 외곽선으로, 등고선은 `contour_width_extra_px`만큼 굵어진 선으로 표시한다. 축이나 데이터를 갱신해도 선택 표시가 데이터 위치를 따라가며, 범위를 벗어난 인덱스는 그리지 않는다.

<a id="resident-selection-details"></a>

### 상주 선택 값 조회와 키보드 이동

저수준 `FiggyChart.selection_details()`는 `Promise<string>`을 반환한다. 이 JSON 문자열을 파싱하면 `Config.picked_data.refs`의 첫 참조가 가리키는 값을 확인할 수 있다. 선택이 없거나 지원하지 않는 참조 종류(`contour_level`)이면 JSON 문자열 `"null"`을 반환한다. 시리즈나 컬럼이 없거나 표본 인덱스가 범위를 벗어나면 Promise를 거부한다. 이 메서드는 상주 GPU 풀에서 선택한 표본 값만 읽으며, 선택 판정을 다시 하거나 `Config.picked_points`를 읽지 않는다.

히스토그램 구간을 선택했을 때 파싱한 결과는 다음과 같다.

```json
{
  "identity": { "kind": "histogram_bin", "source_id": "samples", "series_id": "hist", "bin_index": 2 },
  "series_id": "hist",
  "index": 2,
  "bin_start": 1.0,
  "bin_end": 1.5,
  "value": 0.25,
  "orientation": "Vertical"
}
```

`identity`에 `source_id`가 없으면 그 키를 생략한다. `value`는 저장된 막대 높이이며 개수·가중치·밀도·계단 그래프 값 중 무엇이든 될 수 있다. 가로 히스토그램은 `"Horizontal"`을 반환하고 Y 컬럼을 구간 경계로 사용한다. 점 결과에는 `index`, `x`, `y`가 있고, 시리즈에 `point_color_map`이 있으면 `color_value`, `point_size_map`이 있으면 `size_value`가 더해진다. 두 값은 점의 색과 반지름을 정한 원래 값이다. 행렬 결과에는 `x_index`, `y_index`, `x`, `y`, `value`가 있으며 `Edges` 격자에서는 셀 중심 좌표를 보고한다. 벡터 시리즈는 `u`와 `v`를 보고한다. 점 벡터는 점 결과에 `u` / `v`를 더하고, 격자 벡터는 `u` 격자 기준의 행렬 결과에서 `value` 대신 `u` / `v`를 쓴다. 벡터에 `color_map`이 있으면 `color_value`도 들어 있다. 모든 결과에는 `kind` 태그와 필드를 한 객체에 둔 `identity`와 `series_id`가 있다.

`FiggyChart.step_selection(dx, dy)`는 동기 메서드이며 선택이 바뀌었는지를 반환한다. 세로 히스토그램에서는 `dx`가 구간을, `dy`가 방향이 같은 히스토그램 시리즈를 옮기고, 가로 히스토그램에서는 두 역할이 바뀐다. 구간 인덱스는 선택한 시리즈의 경계·값 개수가 허용하는 범위를 넘지 않는다. 점에서는 `dx`가 원본 인덱스를, `dy`가 점이나 선을 그리는 시리즈를 바꾼다. 점 벡터(`Points` 형식의 `Vector`)의 화살표도 산점도의 점처럼 시리즈의 점을 따라 옮긴다. 행렬 셀(격자 벡터의 셀 포함)에서는 `dx`가 X 인덱스를, `dy`가 Y 인덱스를 바꾼다. 등고선 참조는 옮기지 않는다. `selection_details()`와 `step_selection()`은 `<figgy-chart>` 래퍼에는 없는 저수준 WASM 메서드이므로, 호스트는 `selection_details()`가 끝날 때까지 [비동기 호출 중 객체 접근 제한](../renderer/WASM.md#39-async-메서드와-객체-잠금-필독)의 `busy` 토큰을 유지해야 한다.

<a id="draw_style--렌더-스타일-config-선택-키"></a>

## 렌더링 스타일(`draw_style`)

`Config.draw_style`은 `crates/model/src/config.rs`의 `DrawStyle` 열거형이다. `"mode"`와 해당 스타일의 옵션을 같은 객체 안에 저장한다(internally tagged). 키를 생략하면 기본 정밀 모드인 `precise`다. 기본값을 직렬화할 때도 키를 생략하지만 `{ "mode": "precise" }`를 직접 지정해도 된다.
`{ "mode": "sketch" }`는 손그림 스타일을 켠다. 스타일은 차트 전체에 적용하며 시리즈마다 다르게 지정할 수 없다. `sketch`와 `milkyway`는 선·산점도·오차 막대 시리즈만 그리고, 히스토그램·히트맵·등고선·벡터 시리즈는 오류 없이 건너뛴다.

스케치 옵션에는 모두 기본값이 있어 필요한 항목만 지정할 수 있다. `"draw_style": { "mode": "sketch" }`만 넣으면 모든 옵션에 기본값을 사용한다.

| 필드 (`mode: "sketch"`) | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `amplitude_px` | f32 | `1.5` | 선에 수직인 방향으로 흔들리는 폭(px) |
| `wavelength_px` | f32 | `60.0` | 선을 따라 한 번 굽이치는 간격(px) |
| `seed` | u32 | `0` | 같은 설정·데이터에서 같은 패턴을 만드는 시드 |

전체 형태: `"draw_style": { "mode": "sketch", "amplitude_px": 1.5,
"wavelength_px": 60.0, "seed": 0 }`.

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
| `glow` | f32 | `0.55` | 축·눈금·라벨·제목에 더하는 빛 번짐 강도(`0`이면 끔) |
| `nebula` | f32 | `1.0` | 배경 성운 강도 |
| `dust` | f32 | `1.0` | 배경 먼지 밀도 |
| `planet_rim` | f32 | `0.34` | 산점도 행성의 가장자리 빛 강도 |
| `seed` | u32 | `0` | 전역 시드 |

전체 형태: `"draw_style": { "mode": "milkyway", "star_density": 14.0,
"ribbon_width_px": 14.0, "ribbon_intensity": 0.30, "star_scale": 1.0,
"star_brightness": 1.0, "spread_px": 2.5, "structure_scale": 1.0,
"faint_bias": 3.0, "glow": 0.55, "nebula": 1.0, "dust": 1.0,
"planet_rim": 0.34, "seed": 0 }`.

별자리 스타일인 `constellation`은 `ScatterLine` 시리즈만 지원한다. 데이터 점 위치에 점 확산 함수(PSF)로 빛이 번진 별을 그리고 반투명 선으로 잇는다. 다른 종류의 시리즈는 오류 없이 그리지 않는다.

| 필드 (`mode: "constellation"`) | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `star_opacity` | f32 | `1.0` | 별의 불투명도 |
| `line_opacity` | f32 | `0.45` | 연결선의 불투명도 |

전체 형태: `"draw_style": { "mode": "constellation", "star_opacity": 1.0,
"line_opacity": 0.45 }`.

## 기본값 기준 `get_config()` 전체 형식

기본 `Config`에서는 선택 키가 모두 기본값이므로 `draw_style`, `picked_points`, `picked_data`, `colorbar`, `polar`, `overlays`, `subplots`, `active_layer`, `annotations`가 JSON에 나타나지 않는다. 각 키를 생략하는 규칙과 쓰는 형식은 [serde의 JSON 표현 규칙](#serde-표현-규칙-json을-읽을-때-알아야-할-것)에 있다.

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

## 색상 막대와 Z축 스케일(`colorbar`)

`Config.colorbar`는 색상 막대의 모양과 차트의 Z축 범위를 함께 정의한다. 기본 설정에는 없으며 `None`일 때 키를 생략한다. 따라서 Z축이 필요 없는 기존 문서도 그대로 읽을 수 있다.

`axis`는 차트의 네 축과 같은 `AxisOptions`다. 스케일·최소·최대·눈금 간격·라벨·제목의 의미가 차트 축과 같고, 로그축 눈금과 `Power` 라벨도 같은 방식으로 표시한다. 단, `Timestamp` 라벨 형식은 지원하지 않으며 지정하면 숫자로 표시한다.

| 필드 | 타입 | 기본값 | 의미 |
|---|---|---|---|
| `visible` | bool | `true` | `false`이면 색상 막대를 그리지 않고 여백도 두지 않는다. 색상 막대의 범위와 색상표를 쓰는 시리즈는 계속 그린다. |
| `side` | Side | `"Right"` | `Left`/`Right` = 수직 바, `Top`/`Bottom` = 수평 바. 이것만으로 방향이 결정된다 |
| `thickness_px` | f32 | `18.0` | 색상 막대 두께 |
| `gap_px` | f32 | `24.0` | 데이터 영역과 색상 막대 사이의 간격(px) |
| `length_frac` | f32 | `0.75` | 데이터 영역에서 막대를 놓은 변의 길이에 대한 막대 길이 비율. `(0, 1]` |
| `align` | BarAlign | `"Center"` | 변을 따라 시작·가운데·끝 중 어디에 놓을지 지정 |
| `offset_x`, `offset_y` | f32 | `0.0` | 기준 위치에서의 이동량(px). 드래그 결과를 누적한다. 여백 계산에 반영하지 않으므로 데이터 영역은 그대로다. |
| `colormap` | ColorMap | `"Viridis"` | 값에 따라 연속적으로 색을 지정하는 색상표 |
| `nan_color` | Color | 완전 투명 | 색으로 변환할 수 없는 Z값(NaN, ±무한대, 로그 스케일의 0 이하 값)에 사용할 색 |
| `border_color` | Color | 회색(80,80,80) | 색상 막대 테두리 색 |
| `border_width` | f32 | `1.0` | 색상 막대 테두리 두께(px). `0`이면 테두리를 그리지 않는다 |
| `axis` | AxisOptions | 아래 | Z축 범위와 눈금·라벨 설정. 기본 범위 `0..1`은 임시값이므로 `min` / `max`를 데이터에 맞게 지정하거나 웹에서 `auto_fit_colorbar(pad)`를 호출한다 |

색상 막대는 다음 규칙을 따른다.

- `Heatmap` / `Contour` / `HeatmapContour`, `point_color_map`이 있는 산점도 계열 시리즈, `color_map`이 있는 `Vector`에는 이 설정이 반드시 필요하다. Z축 범위와 색상표를 여기에서 읽으므로 없으면 시리즈를 거부한다.
- 차트당 Z축 스케일은 하나이며 색상 막대를 쓰는 모든 시리즈(행렬, 색상 매핑 산점도, 색상 매핑 벡터)가 같은 범위와 색상표를 공유한다. 웹의 `auto_fit_colorbar(pad)`는 이 시리즈들이 매핑하는 모든 컬럼(행렬 값 컬럼, `point_color_map` 컬럼, 점 벡터 `color_map`의 컬럼, 격자 벡터 `color_map`의 모든 컬럼) 통계의 합집합으로 Z축을 맞춘다.
- 색상 막대의 영역은 `gap_px + thickness_px + axis.out_margin + axis.major_tick_length`이며 해당 방향의 여백에 추가된다. Rust 모델의 `Config::fit_to_data_area`(`FitStrategy::ProportionalScale`)와 `Config::resize_chart_area_scaled`는 이 가운데 라벨 여백인 `axis.out_margin`만 비율에 맞춰 조절하고 `thickness_px`, `gap_px`, 눈금 길이는 유지한다.
- 일반 축과 다른 기본값은 `line_visible: false`, `tick: "Outside"`, `title_option.visible: false`, `out_margin: 60.0`이다. 막대 테두리가 축선 역할을 하고 눈금은 라벨 공간에 놓이며, 기본 여백에는 제목 없이 눈금 라벨만 들어간다. `tick`은 `None` / `Outside` / `Inside` / `Both`로 눈금 방향을, `inverted`는 화면상의 값 증가 방향을 정한다. 눈금 모양은 축선의 `line_color` / `line_width` / `line_style`을 따른다.
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

히스토그램과 행렬 기반 세 종류의 `render_type` 값 예시는 아래와 같다. JSON 최상위 키(`histogram`, `heatmap`, `contour`, `heatmap_contour`)는 예시를 구분하는 이름이며, 각 키의 값을 `SeriesConfig.render_type`에 넣는다. `Histogram`은 호스트가 준비한 `(edges, values)` 컬럼을 받고, 행렬은 `MatrixRef`로 격자를 지정한다. 이 네 종류는 정밀 모드에서만 그리며 다른 [렌더링 스타일](#draw_style--렌더-스타일-config-선택-키)에서는 오류 없이 건너뛴다.

- `Histogram`의 컬럼 역할은 `bar.orientation`으로 정한다. `"Vertical"`이면 `x_column`이 경계, `y_column`이 구간별 값(빈도·가중치·밀도 등)이며 `"Horizontal"`은 반대다. `edges = counts + 1` 같은 길이 관계로 추측하지 않는다.
- 기존 히스토그램을 가로로 바꿀 때는 `bar.orientation = "Horizontal"`과 함께 `x_column = counts`, `y_column = edges`로 연결한다. X축은 빈도, Y축은 구간 범위로 설정하고 축 제목도 바꾼다. 방향만 바꾸고 컬럼 연결을 유지하면 역할이 뒤바뀐다. Rust 예제와 갱신 순서는 [히스토그램 사용법](../renderer/HISTOGRAM.md)을 참고한다.
- `bar.width_ratio`는 구간 대비 막대 너비이며 `0..=1`로 제한한다. 키를 생략하면 `1.0`(구간 전체)이다. 막대를 가운데에 놓고 `gap_px`를 양쪽으로 나누어 추가 간격을 둔다. 1픽셀보다 넓은 막대는 최소 1픽셀이 남도록 간격을 제한한다.
- 구간 자체가 1픽셀보다 좁으면 GPU가 픽셀 열별 최댓값을 골라 0까지 채운다. 가로 히스토그램은 픽셀 행을 기준으로 한다. 이때 간격과 양수인 너비 비율은 적용하지 않는다. 최댓값 구간의 외곽선 두께와 불투명도가 모두 양수이면 외곽선 색으로, 아니면 채움색으로 그린다. 동률이면 앞선 구간을 선택한다. `width_ratio: 0`인 구간은 제외한다.
- `border_width: 0`이면 외곽선을 숨긴다. 양수이면 지정한 두께와 `border_color`를 사용한다.
- `bar.bar_style_overrides`는 `index`로 구간을 골라 `fill_color`, `border_color`, `border_width`, `gap_px`, `width_ratio` 중 필요한 값만 바꾼다. 같은 인덱스가 여러 번 나오면 선언 순서대로 적용한다. 기준선과 방향은 시리즈 전체에 적용한다. 그리기·데이터 피킹·선택 테두리는 같은 최종 막대 경계를 사용한다.
- `MatrixRef.columns`는 격자를 구성하는 컬럼 ID 목록이며, 각 컬럼은 미리 등록해야 한다. 등록되지 않은 컬럼이 있으면 시리즈를 거부한다. 행렬 시리즈의 `x_column` / `y_column`은 격자 좌표 컬럼이다. `orientation: "ColumnsAreX"`이면 각 컬럼이 X 좌표 하나에 해당하고 그 위치의 Y 방향 값을 담으며, `"ColumnsAreY"`이면 각 컬럼이 Y 좌표 하나에 해당하고 X 방향 값을 담는다.
- `MatrixRef.grid_layout`은 좌표가 셀 경계(`"Edges"`, n+1개)인지 중심(`"Centers"`, n개)인지 지정한다. 길이로 추측하지 않는다.
- 선언과 데이터 크기가 다르면 오류 없이 공통으로 사용할 수 있는 범위까지만 그린다. 실제로 그린 개수와 잘림 여부는 `series_draw_info(series_id)`가 반환하는 `drawn_count`, `cols`, `rows`, `truncated`로 확인한다. `cols` / `rows`는 행렬 시리즈와 격자 벡터에서만 값이 있고 다른 종류에서는 `null`이다.
<!-- contour-contract: scope=schema max-levels=1024 -->
- `ContourConfig.levels`에는 데이터 단위의 등고선 값을 명시한다. 레벨을 자동으로 고르려면 웹의 `set_contour_nice_levels(series_id, target_count, use_colormap_colors)`를 호출한다. 색상 막대 Z축 범위 안에서 눈금 규칙으로 고른 값을 이 배열에 기록하고 레벨 수를 반환하며, `use_colormap_colors`가 `true`이면 `per_level_color`도 색상표의 색으로 채운다. `0..=1024`개를 허용하며 1025개 이상이면 `set_series`가 실패한다. 배열을 잘라 처리하지 않고 이전 설정·시리즈·GPU 스타일을 유지한다.
- `ContourLabelConfig.spacing_px`는 자동 배치의 목표 간격이다. 레벨 누락을 막기 위해 추가로 고르는 후보는 더 가까울 수 있다. 라벨을 숨겼거나 위치를 직접 지정했더라도 간격은 유한한 양수여야 한다.
- 자동·직접 배치는 모두 최대 1024개를 사용한다. 직접 지정한 `anchors`에서는 잘못된 `level_index`를 제외하고 입력 순서대로 유효한 앞 1024개를 사용한다. 남은 항목이 없으면 자동 배치하고, 하나라도 있으면 그 목록을 사용한다.
- 자동 배치 간격에는 출력 배율(예: PNG 2x 내보내기의 2)을 곱하며, 곱한 값도 유한한 양수여야 한다. 라벨 글자를 담는 텍스처(아틀라스)는 WebGPU 어댑터의 최대 텍스처 크기를 넘을 수 없다. 두 조건 중 하나를 어기면 렌더링이나 PNG 내보내기가 해당 시리즈의 오류를 반환한다. 앵커는 데이터 좌표 `(x, y)`와 접선 `(tx, ty)`로 저장하며 확대·이동 시 화면에 다시 투영한다.
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

## `Vector` render_type의 전체 형식

점 벡터(`Points`)와 격자 벡터(`Grid`)의 `render_type` 값 예다. 최상위 키 `flow`와 `gradient`는 예시를 구분하는 이름이다. 두 형식 모두 시리즈의 `x_column` / `y_column`이 화살표의 기준점이다. `Points`의 `u` / `v`는 기준점 컬럼과 인덱스가 맞는 컬럼이고, `Grid`의 `u` / `v`는 히트맵 행렬과 같은 방식으로 좌표 컬럼 위에 선언한 격자다. `flow`에는 선택 키를 모두 적었다. `gradient`는 `get_series()`가 반환하는 형태로, `pivot`은 기본값 `"Tail"`이어도 기록하고 `0.0`인 `min_length_px`는 생략한다. 벡터는 정밀 모드에서만 그리며 스트리밍 소스는 오류로 거부한다. 키별 생략 규칙과 `color_map` 조건은 [serde의 JSON 표현 규칙](#serde-표현-규칙-json을-읽을-때-알아야-할-것), 필드 의미는 [벡터장](https://github.com/shim9610/figgy/blob/master/crates/renderer/CONFIG.md#벡터장)을 참고한다.

```json
{
  "flow": {
    "Vector": {
      "components": { "Points": { "u": "u", "v": "v" } },
      "style": {
        "color": { "r": 0.1, "g": 0.15, "b": 0.25, "a": 1.0 },
        "width_px": 1.8,
        "scale": { "Pixels": { "px_per_unit": 12.0 } },
        "pivot": "Middle",
        "head_length_px": 7.0,
        "head_width_px": 6.0,
        "min_length_px": 2.0,
        "color_map": { "Column": "speed" }
      }
    }
  },
  "gradient": {
    "Vector": {
      "components": {
        "Grid": {
          "u": { "columns": ["gu0", "gu1", "gu2"], "orientation": "ColumnsAreX", "grid_layout": "Centers" },
          "v": { "columns": ["gv0", "gv1", "gv2"], "orientation": "ColumnsAreX", "grid_layout": "Centers" }
        }
      },
      "style": {
        "color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 },
        "width_px": 1.6,
        "scale": { "Data": { "factor": 0.35 } },
        "pivot": "Tail",
        "head_length_px": 6.0,
        "head_width_px": 5.0,
        "color_map": { "Grid": { "columns": ["s0", "s1", "s2"], "orientation": "ColumnsAreX", "grid_layout": "Centers" } }
      }
    }
  }
}
```

<a id="get_series-전체-형태--최대-변형-예시"></a>

## 모든 오차 방향을 포함한 `get_series()` 전체 형식

`LineScatterErrorbarXY`, 대칭·비대칭 `ErrorRef`, 라벨을 모두 포함한 시리즈 하나의 예다. 기본값이면 생략하는 `point_color_map`, `point_size_map`과 선 스타일의 `fill`, `fill_color`, `interpolation`은 이 예에 없다. 다른 시리즈 종류는 해당 종류에 필요한 필드만 사용한다.

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
