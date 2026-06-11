# Config / SeriesConfig — JSON 스키마 레퍼런스

`FiggyChart.get_config()` / `get_series()`가 반환하고 `set_config()` /
`set_series()`가 받는 JSON의 **전체 형태**다. 아래 JSON 블록은 Rust 소스에서
직접 직렬화해 생성했고, 동기화 테스트가 어긋남을 막는다:

```bash
cargo test -p model --features serde --test schema_sync          # 검증
cargo test -p model --features serde --test schema_sync print_schema -- --ignored --nocapture   # 재생성
```

**타입의 진실 원본 (Rust 소스)**

| 트리 | 파일 |
|---|---|
| `Config` (축/타이틀/그리드/범례) | `crates/model/src/config.rs` |
| `SeriesConfig` (시리즈 선언) | `crates/model/src/data_config.rs` |
| `Color` | `crates/model/src/color.rs` |
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
  **그려진 수평선**이다. 범례 심볼은 이 조합으로 어떤 형태든 정확히
  같은 길이(2.0 em)가 된다: 선 = `{"text":"—","rule":true,"field_em":2.0,
  "color":{...}}`, 점 = `{"text":"●","field_em":2.0,...}`, 선+점 =
  rule(0.65) + 글리프(0.7) + rule(0.65).
- **색은 0..1 float RGBA**: `{ "r": 0.8, "g": 0.1, "b": 0.1, "a": 1.0 }`.
- `label: null` 가능 (`Option`), 세그먼트 오버라이드 키는 생략 가능 — 그 외
  필드는 전부 항상 존재한다. 부분 업데이트가 아니라 **전체 트리 교체**
  이므로, `get_config()` 결과를 고쳐서 되돌리는 패턴을 쓸 것.

## enum 허용값

| enum | 값 |
|---|---|
| `scale` (AxisScale) | `"Linear"` `"Logarithmic"` |
| `tick` (TickVisibility) | `"None"` `"Outside"` `"Inside"` `"Both"` |
| `format` (LabelFormat) | `"Decimal"` `"Scientific"` `"Power"` |
| `line_style` (LineStylePreset) | `"Solid"` `"Dash"` `"Dot"` `"DashDot"` `"DashDotDot"` `"ShortDash"` `"ShortDot"` `"ShortDashDot"` `"LongDash"` `"LongDashDot"` `"LongDashDotDot"` |
| `corner` (LegendCorner) | `"TopLeft"` `"TopRight"` `"BottomLeft"` `"BottomRight"` |
| `point_shape` (ScatterShape) | `"Circle"` `"Square"` `"Triangle"` `"Diamond"` `"Cross"` `"CircleFilled"` `"SquareFilled"` `"TriangleFilled"` `"DiamondFilled"` |
| `chart_type` (ChartType) | `"ScatterLine"` |
| `render_type` (DataRenderType, 태그) | `"Scatter"` `"Line"` `"ScatterLine"` `"ScatterErrorbarX"` `"ScatterErrorbarY"` `"ScatterErrorbarXY"` `"LineScatterErrorbarX"` `"LineScatterErrorbarY"` `"LineScatterErrorbarXY"` |
| `err_x` / `err_y` (ErrorRef, 태그) | `"Symmetric"` (`{column}`) / `"Asymmetric"` (`{lower, upper}`) |

## 편집 시 의미 결합 주의

- `scale`을 바꾸면 `major_spacing` 해석도 바뀐다 — Linear는 데이터 단위,
  Logarithmic은 **decade 단위** (예: `1.0` = 한 자릿수마다 major 틱).
  데이터 범위가 1 decade 미만이면 decade 틱이 0개일 수 있다.
- `min` / `max`: Logarithmic에서는 양수만.
- `out_margin`을 줄이면 라벨/타이틀이 잘릴 수 있다 (레이아웃 기여 마진).
- `line_offset`은 분리 축 오프셋 — 레이아웃 비기여, 데이터 영역 불변.
- `inverted`는 예약 필드 (미구현).
- `chart_area`는 캔버스 픽셀 기준 패널 사각형 — 보통 `resize()`가 관리하므로
  직접 만지지 말 것.
- `set_series` / `set_series_label` / `apply_color_cycle` 은 범례를 시리즈
  레지스트리에서 **자동 재구성**한다 — 심볼 색은 시리즈 색을 따라온다.
  단 이 재구성은 `legend.content` 의 **세그먼트를 통째로 다시 쓰므로**,
  `set_config` 로 직접 편집한 content (한 줄 배치, 중간 심볼 등) 는 이후의
  시리즈 변경에 덮일 수 있다. 직접 편집 모드로 쓰려면 시리즈 변경 후
  content 를 다시 적용할 것. (`content.font` / `font_size` / `color` 문서
  속성은 재구성에서도 보존된다.)

## `get_config()` 전체 형태 — 기본값 기준

```json
{
  "chart_area": {
    "x": 0,
    "y": 0,
    "width": 1000,
    "height": 800
  },
  "chart": {
    "chart_id": "chart",
    "chart_type": "ScatterLine"
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

## `get_series()` 전체 형태 — 최대 변형 예시

`LineScatterErrorbarXY` + 두 가지 `ErrorRef` 형태 + 라벨이 모두 포함된
한 개짜리 배열. 실제 값은 이 형태의 부분집합 변형들이다.

```json
[
  {
    "series_id": "example",
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
          "point_size": 4.0
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
          "cap_width": 1.0
        }
      }
    }
  }
]
```
