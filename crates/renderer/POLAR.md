# Polar plots

A polar plot is an ordinary Cartesian chart whose `Config.polar` is `Some`:
the same line, scatter and area series, the same column pool, picking and
export, drawn in (angle, radius). `bottom_x` is the angle axis and `left_y`
the radius axis, so ranges, scales, ticks, label styles and titles keep
working; the grid becomes rings and spokes, the angle axis the outer ring and
the radius axis a labelled ray. `polar: None` (the default, omitted in JSON)
is the rectangular chart, so existing documents are unchanged.

Antenna and microphone patterns, wind roses, phase and direction data, orbits
and periodic signals over a cycle are the use cases. For a few series over a
fixed set of categories, drawn as closed polygons, use a
[radar chart](RADAR.md) instead.

## Input and first render

```rust,no_run
use renderer::{
    Chart, Color, Renderer, SeriesConfig,
    data::Column,
    data_config::{DataLineStyleConfig, DataRenderType, LineFill, LineInterpolation},
    default,
    format::LabelFormat,
    layout::{ChartArea, Rect},
    line::LineStylePreset,
    polar::PolarOptions,
};

fn draw(renderer: &mut Renderer) -> Result<(), Box<dyn std::error::Error>> {
    // The columns stay the host's (angle, radius): no projection before upload.
    let degrees: Vec<f64> = (0..=360).map(f64::from).collect();
    let gain: Vec<f64> = degrees
        .iter()
        .map(|d| 0.15 + 0.85 * (2.0 * d.to_radians()).cos().abs())
        .collect();
    renderer.add_column("deg", &Column { data: degrees, min: 0.0, max: 360.0 })?;
    renderer.add_column("gain", &Column { data: gain, min: 0.0, max: 1.0 })?;

    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect { x: 0, y: 0, width: 960, height: 640 });
    config.polar = Some(PolarOptions::default()); // 0° on the right, counter-clockwise
    config.bottom_x.major_spacing = 30.0; // spokes every 30°
    config.bottom_x.label_style.format = LabelFormat::Degrees; // "30°"
    config.left_y.major_spacing = 0.25; // rings every 0.25
    let mut chart = Chart::new(config).with_title("Antenna pattern");
    chart.set_x_range(0.0, 360.0); // the angle axis
    chart.set_y_range(0.0, 1.0); // the radius axis

    let series = [SeriesConfig {
        series_id: "gain".into(),
        source_id: None,
        label: None,
        x_column: "deg".into(),
        y_column: "gain".into(),
        render_type: DataRenderType::Line {
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::from_rgb8(30, 110, 210),
                line_width: 2.5,
                fill: Some(LineFill::ToBaseline(0.0)), // filled to the centre
                fill_color: Some(Color::new(0.12, 0.43, 0.82, 0.25)),
                interpolation: LineInterpolation::Linear,
            },
        },
    }];
    let image = renderer.export_panel_rgba(&chart, &series, 1.0)?;
    std::fs::write("antenna.png", renderer::encode_png(&image)?)?;
    Ok(())
}
```

- `x_column` is the angle in `polar.angle_unit` (`Degrees` by default, or
  `Radians`), `y_column` the radius. `set_x_range` and `set_y_range` set the
  angle and radius axes; the radius axis may be logarithmic, the angle axis
  must be linear and not inverted (direction comes from `clockwise`).
- The plot is the largest circle inside the data area that leaves one band of
  angle labels inside it; `Config::polar_geometry` returns its centre and
  radius. The data area, chart title, legend and margins follow the
  rectangular rules.
- Scatter, line, scatter-line and area series draw in polar mode. Histograms,
  heatmaps, contours, error bars, vector fields, overlay layers and streamed
  sources have no polar form and are rejected with an explicit error
  (`InvalidSeriesConfig` / `InvalidConfig`), never drawn as a fallback.

## Coordinate options

| Option | Effect |
|---|---|
| `zero_angle_degrees` | Where angle 0 points on screen: 0 right (default), 90 up. A compass rose uses 90 |
| `clockwise` | Angles grow clockwise on screen (`true`) or counter-clockwise (`false`, default) |
| `angle_unit` | `Degrees` (default) or `Radians` for the angle column and the sector |
| `sector` | `Some([start, end])` shows only that angular window, at most one turn; drawing is clipped at its edges. `None` is the full circle |
| `inner_radius_fraction` | A hole at the centre, `0.0..=0.9` of the outer radius; `left_y.min` sits on that ring |
| `interpolation` | `Chord` (default): straight segments between samples. `Arc { pieces }`: each segment bends along the data in `pieces` chords, so a constant radius follows its ring and sparse samples still draw a spiral |
| `radial_label_angle_degrees` | The ray the radius axis line and its labels sit on, default 90 (up); dragging the ray turns it |

Every option is additive with a default, so `{ "polar": {} }` in JSON turns a
chart polar. The full field reference, validation rules and clipping details
are in the [configuration reference](CONFIG.md#polaroptions-configpolar).

What is and is not drawn:

- In the full circle, an angle outside the axis range lands where `cos` /
  `sin` put it; the renderer does not wrap, sort or deduplicate samples. In a
  sector, drawing is cut at the sector edges.
- A radius below `left_y.min` is not drawn and a line breaks there. A radius
  above `left_y.max` (a zoomed-in radius axis) is cut at the outer ring, like
  the data-area scissor cuts a rectangular chart. Nothing is drawn inside
  the hole.
- Dashes walk the drawn path, including the pieces of an `Arc` segment; a
  line fill fills to the baseline ring (`LineFill::ToBaseline`), the centre
  for a baseline at the axis start.

## Axes, grid and labels

`grid.show_major_y` / `show_minor_y` draw rings at the radius ticks and
`show_major_x` / `show_minor_x` spokes at the angle ticks, in the grid
colours and widths. `bottom_x` draws the outer ring with its ticks pointing
outward and its labels around it; `left_y` draws the ray at
`radial_label_angle_degrees` with its ticks and labels beside it. The tick
values come from the same nice-tick computation as rectangular axes, so
`major_spacing`, `minor_count`, `significant_digits` and `format` apply.
`LabelFormat::Degrees` writes `90°`, `LabelFormat::Radians` writes `π/2`,
`3π/4` and `0.4π`. `top_x` and `right_y` are not drawn.

## Picking and selection

`pick_nearest_point` and the GPU pick path project through the same mapping
as the shaders, so markers, chords and arc pieces are picked where they are
drawn, and the CPU and GPU pickers agree. A marker outside the sector is not
picked; a line's pieces are pickable where they draw. Hover, click selection
and `step_selection` work as on a rectangular chart.

## Decorations and dragging

The title, the legend and the data area drag and resize with the rectangular
rules. The axes follow the polar frames instead:

- The outer ring (`"axis_bottom"`) is the data boundary and registers no hit
  region; `line_offset` has no polar meaning.
- The radius axis ray (`"axis_left"`) turns around the centre when dragged:
  with the pointer position (`Draggable::drag_from`, what the browser
  wrapper's `on_move` sends) it follows the pointer's angle exactly; without
  one (`drag_by`, keyboard nudges) a point on the outer ring follows the
  drag. The angle lands in `radial_label_angle_degrees`.
- The angle tick labels (`"tick_labels_bottom"`) move along their own spokes,
  outward or inward together: `bottom_x.label_style.label_offset_y` is the
  radial offset in polar mode (positive outward) and `label_offset_x` has no
  effect.
- The radius tick labels (`"tick_labels_left"`) move in the ray's frame:
  `left_y.label_style.label_offset_x` along the ray (outward positive) and
  `label_offset_y` across it, so they keep their place when the ray turns.
  The text itself stays upright.

Annotations draw around the plot with ratio anchors; a data anchor is
reported as a skip in the first version.

## Subplots and export

A subplot's `config.polar` makes that panel polar, so polar and rectangular
panels mix in one document. PNG export at any scale goes through the same
`export_panel_rgba` / `export_document_rgba` calls as rectangular charts, and
rings stay round at 2x. The browser wrapper's `FiggyChart` accepts `polar`
in its config JSON (see the [WASM schema](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)).

## Examples and tests

```bash
cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
cargo test --locked -p figgy-renderer --features serde --test polar_render
```

`polar_plot` writes four PNGs: the antenna pattern above, a compass rose
(clockwise from the top, arc segments, markers), a dashed spiral from 24
sparse samples with arc interpolation, and a half-circle sector in radians
with a hole in the middle. The playground scene "Polar" shows the same chart
interactively.

## 한국어 사용법

극좌표 플롯은 `Config.polar`가 `Some`인 보통의 직교 차트다. 같은 선·산점도·면 시리즈, 같은
컬럼 풀, 피킹과 내보내기를 (각도, 반지름)으로 그린다. `bottom_x`가 각도 축, `left_y`가 반지름
축이라 범위·스케일·눈금·라벨 스타일·제목이 그대로 동작하고, 격자는 고리와 스포크, 각도 축은
바깥 고리, 반지름 축은 라벨이 붙은 광선이 된다. `polar: None`(기본, JSON에서 생략)은 직교
차트이므로 기존 문서는 바뀌지 않는다. 고정된 범주 위에서 시리즈 몇 개를 닫힌 다각형으로
비교하려면 [레이더 차트](RADAR.md)를 쓴다.

### 입력과 첫 렌더링

위 영어 예제처럼 (각도, 반지름) 컬럼을 그대로 올리고, `config.polar = Some(PolarOptions::default())`로
켠 뒤 `set_x_range`로 각도 축, `set_y_range`로 반지름 축 범위를 정한다. `x_column`은
`polar.angle_unit` 단위의 각도(기본 `Degrees`, 또는 `Radians`), `y_column`은 반지름이다. 반지름
축은 로그 스케일이 가능하고, 각도 축은 선형이며 뒤집을 수 없다(방향은 `clockwise`로 정한다).
플롯은 데이터 영역 안에서 각도 라벨 띠 하나를 남기는 가장 큰 원이며 `Config::polar_geometry`가
중심과 반지름을 돌려준다. 산점도·선·산점선·면 시리즈를 그리고, 히스토그램·히트맵·등고선·오차
막대·벡터·오버레이·스트리밍 소스는 명시적 오류로 거부한다.

### 좌표 옵션

| 옵션 | 효과 |
|---|---|
| `zero_angle_degrees` | 각도 0이 가리키는 화면 방향. 0 오른쪽(기본), 90 위. 나침반은 90 |
| `clockwise` | 각도가 화면에서 시계 방향으로 커지는지(`true`) 반시계인지(`false`, 기본) |
| `angle_unit` | 각도 컬럼과 섹터의 단위. `Degrees`(기본) 또는 `Radians` |
| `sector` | `Some([start, end])`이면 그 각도 창만 보이며 경계에서 클립된다. 한 바퀴 이하. `None`은 전체 원 |
| `inner_radius_fraction` | 중심 구멍. 바깥 반지름의 `0.0..=0.9`이며 `left_y.min`이 그 고리에 놓인다 |
| `interpolation` | `Chord`(기본): 표본 사이가 직선. `Arc { pieces }`: 세그먼트가 데이터를 따라 `pieces`개의 현으로 휘어 반지름이 일정하면 고리를 따르고 드문 표본으로도 나선이 그려진다 |
| `radial_label_angle_degrees` | 반지름 축 선과 라벨이 놓이는 광선. 기본 90(위). 광선을 드래그하면 바뀐다 |

모든 옵션에 기본값이 있어 JSON에서 `{ "polar": {} }`만으로 극좌표가 켜진다. 전체 필드와 검증,
클립 규칙은 [설정 명세](CONFIG.md#polaroptions-configpolar)에 있다. 전체 원에서는 축 범위
밖 각도가 `cos`/`sin`이 두는 자리에 그려지고(감싸기·정렬·중복 제거 없음), 섹터에서는 경계에서
잘린다. `left_y.min`보다 작은 반지름은 그리지 않고 선이 끊기며, `left_y.max`보다 큰 반지름은
바깥 고리에서 잘린다. 구멍 안에는 아무것도 그리지 않는다. 대시는 호 조각을 포함해 그려진 경로를
따라가고, 면 채움은 기준선 고리까지 채운다.

### 축·격자·라벨

`grid.show_major_y` / `show_minor_y`는 반지름 눈금의 고리를, `show_major_x` / `show_minor_x`는
각도 눈금의 스포크를 그린다. `bottom_x`는 바깥 고리와 바깥쪽 눈금, 둘레의 라벨을, `left_y`는
`radial_label_angle_degrees` 방향의 광선과 그 옆의 눈금·라벨을 그린다. 눈금 값은 직교 축과
같은 계산이라 `major_spacing`, `minor_count`, `significant_digits`, `format`이 그대로 적용된다.
`LabelFormat::Degrees`는 `90°`, `LabelFormat::Radians`는 `π/2`, `3π/4`, `0.4π`로 쓴다.
`top_x`와 `right_y`는 그리지 않는다.

### 피킹과 선택

`pick_nearest_point`와 GPU 피킹은 셰이더와 같은 매핑으로 투영하므로 마커·현·호 조각이 그려진
자리에서 골라지고 CPU와 GPU 결과가 일치한다. 섹터 밖 마커는 고르지 않고, 선 조각은 그려진 곳에서
고를 수 있다. 호버·클릭 선택·`step_selection`은 직교 차트와 같다.

### 장식 요소와 드래그

제목·범례·그리기 영역은 직교 규칙 그대로 움직이고 크기를 바꾼다. 축은 극좌표 프레임을 따른다.

- 바깥 고리(`"axis_bottom"`)는 데이터 경계라 히트 영역이 없고 `line_offset`은 의미가 없다.
- 반지름 축 광선(`"axis_left"`)은 드래그하면 중심을 축으로 돈다. 포인터 위치가 있으면
  (`Draggable::drag_from`, 웹 래퍼의 `on_move`) 포인터 각도를 그대로 따르고, 없으면(`drag_by`,
  키보드 이동) 바깥 고리 위의 점이 드래그를 따라간다. 결과는 `radial_label_angle_degrees`에
  기록된다.
- 각도 눈금 라벨(`"tick_labels_bottom"`)은 각자 자기 스포크를 따라 함께 안팎으로 움직인다.
  극좌표 모드에서 `bottom_x.label_style.label_offset_y`가 반경 오프셋(양수가 바깥)이고
  `label_offset_x`는 효과가 없다.
- 반지름 눈금 라벨(`"tick_labels_left"`)은 광선 프레임에서 움직인다.
  `left_y.label_style.label_offset_x`가 광선 방향(바깥이 양수), `label_offset_y`가 수직 방향이라
  광선이 돌아도 같은 자리를 지킨다. 글자는 수평을 유지한다.

주석은 비율 앵커로 플롯 둘레에 그리며, 데이터 앵커는 첫 버전에서 건너뛴 것으로 보고한다.

### 서브플롯과 내보내기

서브플롯의 `config.polar`로 그 패널만 극좌표가 되므로 한 문서에 극좌표 패널과 직교 패널을 섞을 수
있다. PNG 내보내기는 직교 차트와 같은 `export_panel_rgba` / `export_document_rgba`를 쓰며 2배
배율에서도 고리가 둥글다. 브라우저 래퍼 `FiggyChart`는 설정 JSON의 `polar`를 받는다
([WASM 스키마](https://github.com/shim9610/figgy/blob/master/crates/web/SCHEMA.md)).

### 예제와 테스트

```bash
cargo run --locked -p figgy-renderer --example polar_plot -- target/polar-plot
cargo test --locked -p figgy-renderer --features serde --test polar_render
```

`polar_plot`은 PNG 네 장을 만든다. 안테나 패턴, 나침반(위에서 시계 방향, 호 세그먼트, 마커),
드문 표본 24개로 그린 점선 나선(호 보간), 가운데 구멍이 있는 라디안 반원 섹터다. 플레이그라운드의
"Polar" 장면은 같은 차트를 상호작용으로 보여준다.
