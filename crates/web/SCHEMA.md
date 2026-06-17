# Config / SeriesConfig — JSON 스키마 레퍼런스

`FiggyChart.get_config()` / `get_series()`가 반환하고 `set_config()` /
`set_series()`가 받는 JSON의 **전체 형태**다. 아래 JSON 블록은 Rust 소스에서
직접 직렬화해 생성했고, 동기화 테스트가 어긋남을 막는다:

```bash
cargo test -p model --features serde
```

`schema_sync` 통합 테스트는 제거되었다. 이 문서는 `crates/model`의 serde
형태와 default 값을 기준으로 수동 갱신한다.

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
  **그려진 수평선**이다. `rule_dash` (선택 키) 는 rule 전용 dash/gap
  패턴이며 em 단위라 폰트 크기와 함께 스케일된다. 범례 심볼은 이
  조합으로 어떤 형태든 정확히 같은 길이(2.0 em)가 된다: 선 =
  `{"text":"—","rule":true,"field_em":2.0,"color":{...}}`, 점 =
  `{"text":"●","field_em":2.0,...}`, 점선 =
  `{"text":"—","rule":true,"field_em":2.0,"rule_dash":[0.571,0.286],...}`,
  선+점 = rule(0.65) + 글리프(0.7) + rule(0.65).
- **색은 0..1 float RGBA**: `{ "r": 0.8, "g": 0.1, "b": 0.1, "a": 1.0 }`.
- `label: null` 가능 (`Option`), 세그먼트 오버라이드 키와 Config의
  `draw_style` 키는 생략 가능 (`precise` = 키 자체가 생략, 아래
  [`draw_style` 절](#draw_style--렌더-스타일-config-선택-키) 참고) — 그 외
  필드는 전부 항상 존재한다. 부분 업데이트가 아니라 **전체 트리
  교체**이므로, `get_config()` 결과를 고쳐서 되돌리는 패턴을 쓸 것.

## enum 허용값

| enum | 값 |
|---|---|
| `scale` (AxisScale) | `"Linear"` `"Logarithmic"` |
| `tick` (TickVisibility) | `"None"` `"Outside"` `"Inside"` `"Both"` |
| `format` (LabelFormat) | `"Decimal"` `"Scientific"` `"Power"` |
| `line_style` (LineStylePreset) | `"Solid"` `"Dash"` `"Dot"` `"DashDot"` `"DashDotDot"` `"ShortDash"` `"ShortDot"` `"ShortDashDot"` `"LongDash"` `"LongDashDot"` `"LongDashDotDot"` |
| `corner` (LegendCorner) | `"TopLeft"` `"TopRight"` `"BottomLeft"` `"BottomRight"` |
| `point_shape` (ScatterShape) | `"Circle"` `"Square"` `"Triangle"` `"Diamond"` `"Cross"` `"CircleFilled"` `"SquareFilled"` `"TriangleFilled"` `"DiamondFilled"` `"TriangleDown"` `"TriangleLeft"` `"TriangleRight"` `"Plus"` `"Pentagon"` `"Hexagon"` `"Octagon"` `"Star"` `"TriangleDownFilled"` `"TriangleLeftFilled"` `"TriangleRightFilled"` `"PlusFilled"` `"CrossFilled"` `"PentagonFilled"` `"HexagonFilled"` `"OctagonFilled"` `"StarFilled"` |
| `render_type` (DataRenderType, 태그) | `"Scatter"` `"Line"` `"ScatterLine"` `"ScatterErrorbarX"` `"ScatterErrorbarY"` `"ScatterErrorbarXY"` `"LineScatterErrorbarX"` `"LineScatterErrorbarY"` `"LineScatterErrorbarXY"` |
| `err_x` / `err_y` (ErrorRef, 태그) | `"Symmetric"` (`{column}`) / `"Asymmetric"` (`{lower, upper}`) |

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

## `get_series()` 전체 형태 — 최대 변형 예시

`LineScatterErrorbarXY` + 두 가지 `ErrorRef` 형태 + 라벨이 모두 포함된
한 개짜리 배열. 실제 값은 이 형태의 부분집합 변형들이다.

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
          "cap_width": 1.0
        }
      }
    }
  }
]
```
