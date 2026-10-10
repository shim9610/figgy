# 원형 차트

`RadialChart`는 소수의 범주와 구성 비율을 보여 주는 파이·도넛 차트다. `RadialRenderer`는
기존 렌더러와 같은 wgpu 장치·큐를 사용할 수 있다. 축, 컬럼 풀, 자동 fit, 스트리밍 작업은
사용하지 않는다. 범주 값과 스타일을 모두 담은 모델을 넘기면 불변 프레임을 만든다.
이 API는 renderer crate의 독립 렌더링 경로이며, 웹 Studio의 차트 생성 메뉴에는 아직 연결하지 않았다.

## API 구성

모델 타입은 `model::radial`에 정의하며 `renderer::radial`과 renderer crate 루트에서도
가져올 수 있다. 모델을 편집한 뒤 `prepare`를 다시 호출하면 된다. 별도의 색 변경 명령이나
캐시 무효화 호출은 필요하지 않다.

| 타입 / 메서드 | 역할 |
|---|---|
| `RadialChart` | 값·모양·공통 스타일·라벨·상호작용 상태의 원본 |
| `RadialSlice`, `RadialSplit` | 조각 데이터와 한 조각을 펼친 상세 원 |
| `RadialStyle`, `RadialOutline` | 재질·입체 형상·질감·외곽선 설정 |
| `RadialLabels`, `RadialLabelFormat` | 라벨 위치와 표시할 내용 |
| `RadialTarget`, `RadialInteraction` | 조각 식별과 호버·선택 상태 |
| `RadialChart::validate()` | 값과 옵션 검사. 실제 라벨 배치는 `prepare`에서 추가 검사 |
| `RadialRenderer::prepare()` | 모델을 읽고 불변 `Arc<RadialFrame>` 반환 |
| `RadialFrame::draw()` | 호스트가 연 render pass에 그리기 |
| `RadialFrame::hit_test()` | 해당 프레임에서 보이는 조각 선택 |
| `export_rgba_async()` / `export_rgba()` | 현재 모델을 그려 `RasterImage` 반환. 동기판은 네이티브 전용 |
| `clear_cache()` / `end_frame()` | 내부 캐시 참조 해제 / 제출이 끝난 자원의 회수 예약 |

`RadialChart::default()`는 빈 파이 차트이므로, 그리기 전에 양수 값을 가진 조각을 추가해야 한다.

## 시작하기

```rust,no_run
use renderer::{
    Color, RadialChart, RadialKind, RadialLabels, RadialMaterial,
    RadialRenderer, RadialSlice, RendererDevice, encode_png,
};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let instance = renderer::data_render::create_instance();
# let adapter = renderer::data_render::request_adapter(&instance)?;
# let (device, queue) = renderer::data_render::request_device(&adapter)?;
# let device = std::sync::Arc::new(device);
# let queue = std::sync::Arc::new(queue);
let mut renderer = RadialRenderer::new(
    RendererDevice::new(device, queue),
    wgpu::TextureFormat::Rgba8Unorm,
)?;
let mut chart = RadialChart {
    title: "Regional share".into(),
    slices: vec![
        RadialSlice::new("A", 30.0, Color::from_rgb8(28, 179, 155)),
        RadialSlice::new("B", 70.0, Color::from_rgb8(251, 128, 74)),
    ],
    kind: RadialKind::Donut { inner_radius: 0.48 },
    labels: RadialLabels::Outside,
    ..Default::default()
};
chart.style.material = RadialMaterial::Ceramic;
chart.style.tilt_degrees = 55.0;
chart.style.depth = 0.18;
chart.style.shadow = true;

// 논리 크기는 1000×600, 실제 출력은 2000×1200이다.
// 글자와 도형을 이 해상도로 다시 그린다.
let image = renderer.export_rgba(&chart, (1000, 600), 2.0)?;
std::fs::write("donut.png", encode_png(&image)?)?;
renderer.end_frame();
# Ok(())
# }
```

화면에 그릴 때는 `prepare(&chart, logical_size, scale)`로 `Arc<RadialFrame>`을 받은 뒤,
호스트의 render pass 안에서 `frame.draw(&mut pass)`를 호출한다. 타깃은 생성자에 지정한
RGBA8/BGRA8 포맷, `frame.size()` 크기, sample count 1이어야 한다. 가장자리 AA는 셰이더가
픽셀당 네 위치를 평가해 처리한다. `scale`은 0.5~4이며 타깃의 실제 크기도 함께 맞춘다.
호스트 render pass의 clear/load 정책은 호스트가 정한다. 투명 배경도 지원한다.
논리 크기는 가로·세로 각각 최소 160픽셀이다. 실제 크기는 `round(논리 크기 × scale)`이며
장치의 텍스처 크기 한도를 넘을 수 없다. `draw`는 viewport와 scissor를 타깃 전체로 설정한다.
UI의 일부 영역에 넣으려면 이 크기의 별도 텍스처에 그린 뒤 호스트가 배치한다.

WASM은 같은 `prepare`/`draw`를 쓰고, 출력에는 `export_rgba_async(...).await`를 사용한다.
동기 `export_rgba`는 네이티브 전용이다. PNG 인코딩은 기존 `encode_png`를 사용한다.

## 값과 분할 원

- 원마다 1~64개 항목을 받는다. 값은 유한한 0 이상의 수여야 하고, 전체 합은 양수여야 한다.
- 0인 항목은 모델에 남지만 도형과 라벨은 만들지 않는다. NaN·무한대·음수·합계 오버플로는 거절한다.
- 조각 색은 불투명 RGBA다. 배경과 라벨에는 알파를 사용할 수 있다.
- 각도는 오른쪽을 0도로 보고 시계 방향으로 증가한다. 기본 시작 각도는 -90도다.
- `explode`는 바깥 반지름 대비 조각 이동량이다. 0~0.35를 지원한다.
- `RadialSplit`은 한 항목을 오른쪽의 작은 원으로 펼친다. 자식 값은 부모와 **같은 단위**이며
  합계가 부모 값과 같아야 한다(상대 오차 1e-9까지 허용). 자식 원의 라벨은 부모 소계 대비 비율이다.

예를 들어 전체 100 중 ‘기타’가 13이고, 세부 비율이 16%·14%·70%라면 자식 값은
`2.08`, `1.82`, `9.1`이다. `16`, `14`, `70`을 그대로 넣으면 합계가 맞지 않아 오류가 난다.
`slice_index`는 0인 항목까지 포함한 원본 배열 인덱스다. 연결선은 선택한 부모 조각의 두
각도 경계와 상세 원을 잇는다. 예제처럼 부모 조각을 오른쪽에 두면 두 원의 관계가 잘 보인다.

## 재질과 입체 표현

모델 기본값은 정면에서 내려다본 평면 파이다. 재질을 바꾸는 것만으로 기울기나 깊이가
바뀌지는 않는다. 갤러리처럼 입체 도넛을 만들려면 `kind`, `tilt_degrees`, `depth`도 지정한다.

| 설정 | 표현 |
|---|---|
| `Flat` | 조명 없이 지정한 색을 그대로 사용하는 평면 표현 |
| `Matte` | 확산광과 약한 표면 입자 |
| `Ceramic` | 매끈한 표면, 밝은 반사광과 둥근 모서리 느낌 |
| `SatinMetal` | 넓게 퍼지는 반사광과 약한 결을 가진 새틴 금속 |
| `BrushedMetal` | 원주 방향의 미세한 결과 방향성 반사 |
| `Toon` | 세 단계 명암. 외곽선은 별도로 켜거나 끈다 |
| `Enamel` | 선명한 코팅 광택 |
| `Hatch` | 대각선 해칭 |
| `Pearl` | 곡면 가장자리의 약한 펄 색 변화 |
| `Paper` | 윗면·옆면에 이어지는 미세한 무광 종이 결 |
| `Wood` | 굽은 결 무늬와 약한 광택 |

`tilt_degrees`(0~65), `depth`(반지름의 0~0.4), `gap_degrees`(0~8), `bevel`(0~0.1),
`roughness`(0.05~1), `texture_strength`(0~1), `texture_scale`(0.1~8), `light`, `shadow`를
조합한다. 아주 작은 조각의 시각적 간격은 해당 조각 각도의 20% 이내로 제한한다.

직교 카메라의 광선과 둥근 조각의 거리 함수를 사용해 윗면·바깥 벽·안쪽 벽·절단면을 그린다.
`inner_corner`와 `outer_corner`는 평면에서 본 조각 끝의 둥근 정도다(바깥 반지름의 0~0.25).
0이면 각진 형태이며 2D·3D에 모두 적용된다. 작은 조각과 얇은 링에서는 반경을 자동으로
제한한다. `bevel`은 입체 조각의 위아래 모서리를 실제로 둥글게 만들며, 깊이의 절반 미만으로
제한한다. 깊이가 0이면 입체 bevel은 적용하지 않는다. 그림자는 조각별 부드러운 투영 근사다. 재질은 외부 이미지 없이 셰이더에서 생성한다. 입체 투영을 사용해도
원래 각도와 비율 라벨은 유지하지만, 화면에 투영된 조각 면적은 기울기에 따라 달라진다.

`gloss`(0~1)는 반사광 강도, `roughness`는 퍼짐, `texture_angle_degrees`는 표면 무늬 방향을
조절한다. 같은 조명에서도 재질에 따라 적용 방식이 다르다. 물리 기반 렌더러 전체를 구현한
것은 아니며, 차트의 범주 색을 유지하는 스타일화된 셰이딩이다.
종이는 높이까지 포함한 연속적인 질감 좌표를 사용해 옆면에 세로줄이 생기지 않도록 한다.
출력 크기와 면의 기울기로 구분할 수 없는 미세 무늬는 약하게 줄여 깜빡임을 억제한다. `Flat`은 광택·질감을 사용하지
않고, `Toon`은 거칠기 대신 고정 명암 단계를 사용한다. `Wood`는 기존 API 호환을 위해 남기되
새 갤러리와 편집기의 추천 목록에는 노출하지 않는다.

### 스타일 옵션 기본값과 범위

비율로 표기한 길이는 **바깥 반지름**을 기준으로 한다. 범위를 벗어난 입력과 유한하지 않은
수는 거절한다. 범위 안에서도 작은 조각의 모서리·간격은 서로 겹치지 않도록 줄어들 수 있다.

| 필드 | 기본값 | 허용 범위 / 의미 |
|---|---|---|
| `material` | `Flat` | 위 재질 목록 |
| `tilt_degrees` | `0` | 0~65°. 0은 위에서 수직으로 내려다보는 시점 |
| `depth` | `0` | 반지름의 0~0.4 |
| `bevel` | `0.015` | 반지름의 0~0.1. 실제 적용은 깊이의 절반 미만 |
| `inner_corner`, `outer_corner` | 각각 `0` | 반지름의 0~0.25 |
| `gap_degrees` | `0` | 0~8°. 조각 각도의 20% 이내 |
| `roughness` | `0.35` | 0.05~1 |
| `gloss` | `0.55` | 0~1 |
| `texture_strength` | `0.2` | 0~1. 0이면 무늬 효과 끄기 |
| `texture_scale` | `1` | 0.1~8. 높일수록 무늬가 촘촘해짐 |
| `texture_angle_degrees` | `0` | 유한한 각도. 360° 주기로 반복 |
| `light` | `[-0.5, -0.6, 1.0]` | 방향 벡터. 성분별 -100~100, 벡터 길이 최소 0.01 |
| `shadow` | `false` | 부드러운 그림자 켜기 |
| `hover_lift` | `0.08` | 반지름의 0~0.25. 호버·선택 시 수직 이동 |
| `hover_brightness` | `0.04` | 0~0.5. 호버·선택 시 밝기 증가 |
| `outline.rim`, `.separators`, `.emphasis` | 모두 `false` | 윤곽·조각 경계·강조선 켜기 |
| `outline.width` | `1` | 논리 픽셀 0~8 |
| `outline.color` | RGBA `(0.08, 0.12, 0.2, 1)` | 성분별 0~1, 알파 허용 |

도넛의 `inner_radius`는 바깥 반지름의 0.1~0.85다. 구멍 없는 모양은 `RadialKind::Pie`로
지정한다. `font_size`는 논리 픽셀 8~64(기본 16), 제목은 한 줄 160자 이내,
`value_suffix`는 한 줄 24자 이내다. 접미사 앞의 공백도 직접 넣는다(예: `" kg"`).

## 라벨과 갱신

`None`, `Inside`, `Outside`를 지원한다. 내부 라벨은 조각의 위치에, 외부 라벨은 좌우로
나누어 연결선과 함께 배치한다. 외부 라벨은 같은 쪽에서 겹치지 않도록 세로 위치를 조정한다.
내부 라벨은 조각 색이 어두우면 흰색을, 그 밖에는 차트의 `label_color`를 사용한다.
조각의 `label_color`를 지정하면 자동 대비 색보다 우선한다.
라벨이 들어갈 공간이 없으면 `prepare`가 오류를 반환한다. 캔버스를 키우거나 외부 라벨을
사용하거나 이름을 줄여야 한다. 항목 이름은 한 줄, 80자 이내다.

같은 모델·논리 크기·배율·글꼴 등록 상태에서는 프레임과 GPU 자원을 재사용한다. 값이나
스타일을 바꾸면 새 불변 프레임을 만든다. 호버·선택·재질·외곽선처럼 라벨 배치를 바꾸지 않는
변경에서는 라벨 텍스처를 공유한다. 외부 라벨의 조각 색 변경도 라벨 재생성을 요구하지 않지만,
내부 라벨의 자동 대비 색이 달라질 수 있는 변경은 라벨을 갱신한다. 이미 받은 프레임은 바뀌지 않으므로 준비 중인 화면과
새 화면의 상태가 섞이지 않는다. 유효하지 않은 입력이나 예산 초과도 기존 프레임을 덮어쓰지 않는다.

라벨은 기존 글꼴 등록과 fallback 경로를 사용한다. 네이티브는 시스템 글꼴을 찾을 수 있지만,
브라우저에서 한글을 표시하려면 기존 `register_font_bytes`로 한글 글꼴을 등록해야 한다.


## SSOT 편집과 선택

색은 `RadialSlice.color`에서 직접 바꾼다. `slice.style: Option<RadialStyle>`은 해당 조각의
스타일 전체를 덮어쓰며, `None`이면 차트 스타일을 상속한다. 개별 스타일을 만든 뒤에는 차트의
스타일 변경이 그 조각에 전파되지 않는다. **기울기·깊이·조명도 포함한 전체 덮어쓰기**이므로,
카메라 각도를 함께 바꾸려면 공통 스타일과 개별 스타일의 `tilt_degrees`를 모두 갱신해야 한다.
다시 상속하려면 `None`으로 돌린다.
`labels`, `label_format`, `label_color`도 조각별로 덮어쓸 수 있다.
`slice.labels = None`은 상속이고, `Some(RadialLabels::None)`은 그 조각의 라벨 숨기기다.

```rust,no_run
use renderer::radial::*;
use renderer::Color;
let mut chart = RadialChart {
    slices: vec![RadialSlice::new("Search", 40.0, Color::from_rgb8(51,132,245))],
    ..Default::default()
};
chart.label_format = RadialLabelFormat::NamePercent;
chart.label_decimals = 1; // 0..6, 불필요한 소수점 0은 제거
chart.value_suffix = " units".into();
let target = RadialTarget::main(0);
let override_style = RadialStyle {
    material: RadialMaterial::SatinMetal,
    inner_corner: 0.05,
    outer_corner: 0.05,
    ..chart.style.clone()
};
let slice = chart.slice_mut(target).unwrap();
slice.color = Color::from_rgb8(195,80,210);
slice.style = Some(override_style);
slice.label_format = Some(RadialLabelFormat::NameValuePercent);
chart.interaction.selected = Some(target);
```

`RadialLabelFormat`은 `Name`, `Value`, `Percent`, `NamePercent`, `NameValue`, `ValuePercent`,
`NameValuePercent`를 제공한다. 표기 내용과 `None/Inside/Outside` 위치를 독립적으로 설정한다.
백분율은 원별 합계, 값은 원래 단위를 사용한다. 세부 원의 백분율은 부모 소계를 기준으로 한다.
표기 변경은 원본 값과 조각 각도를 바꾸지 않는다. 라벨 공간이 부족하면 오류를 반환한다.

`frame.hit_test([x, y])`는 차트 내부의 **논리 픽셀**을 받아 화면에 실제로 보이는 조각의
`RadialTarget`을 반환한다. 기울기·둥근 모서리·간격·들림·가림 관계를 반영한다.
`group=0`은 큰 원, `group=1`은 세부 원이며 `index`는 값이 0인 항목도 포함한 원본 인덱스다.
값이 0인 항목, 구멍, 배경은 선택되지 않는다. 배열을 재정렬하거나 항목을 삭제한 호스트는
선택 인덱스도 함께 갱신해야 한다.

호버는 `interaction.hovered`와 `hover_progress`(0~1), 클릭 선택은 `interaction.selected`로
분리된다. `animate_hover(target, dt_seconds, duration_seconds)`는 시간을 입력받아 진입·이탈을
진행한다. 다른 조각으로 이동할 때는 이전 조각이 내려온 뒤 새 조각이 올라간다. duration이
0이면 즉시 전환한다. 스타일의 `hover_lift`와 `hover_brightness`를 0으로 두면 해당 효과를 끈다.
클릭 선택은 포인터가 나가도 강조를 유지하며, 다시 선택을 해제하면 복원된다.
`hit_test_at_rest`는 들리기 전 위치를 조회한다. 호스트는 이 결과가 직전 호버와 같을 때만
보조 판정에 사용해 가장자리에서 호버가 반복해서 켜지고 꺼지는 것을 막을 수 있다.

외부 라벨은 호버 중 고정된 위치를 유지하고, 내부 라벨은 동일한 라벨 텍스처를 이동해 읽어
조각과 함께 올라간다. 글자를 확대하거나 이전 프레임을 다시 변환하지 않는다. 원래 비율과
차트 크기는 호버로 바뀌지 않는다. PNG는 넘긴 모델의 선택·호버 상태를 그대로 포함한다.
일시 상태 없는 PNG가 필요하면 모델 복사본의 `interaction`을 `Default::default()`로 초기화한다.

`style.outline`의 `rim`(바깥·구멍 윤곽), `separators`(조각 경계), `emphasis`(호버·선택 강조)는
각각 켜고 끌 수 있다. 색·알파·논리 픽셀 두께를 조절하며 숨은 뒷면의 선을 투과해 그리지 않는다.
조각이 맞붙은 공유 경계는 각 조각 안쪽에 반 폭씩 그린다. 라벨과 연결선은 재질 셰이딩을 받지 않는다.

### 호스트의 갱신 순서

포인터 좌표에서 차트의 화면상 원점을 빼고 DPR로 변환해 **차트 내부 논리 좌표**를 구한다.
UI가 이미 논리 좌표를 제공하면 DPR로 다시 나누지 않는다. 현재 표시 중인 프레임으로 선택을
판정하고, 모델의 상호작용 상태를 바꾼 뒤 새 프레임을 준비한다.

```rust,no_run
use renderer::{RadialChart, RadialError, RadialFrame, RadialRenderer};
use std::sync::Arc;

fn update_hover(
    renderer: &mut RadialRenderer,
    chart: &mut RadialChart,
    displayed: &Arc<RadialFrame>,
    pointer: Option<[f32; 2]>, // 차트 내부 논리 좌표. 영역 밖이면 None
    elapsed_seconds: f32,
    logical_size: (u32, u32),
    scale: f32,
) -> Result<Arc<RadialFrame>, RadialError> {
    let target = pointer.and_then(|p| {
        displayed.hit_test(p).or_else(|| {
            displayed.hit_test_at_rest(p)
                .filter(|t| Some(*t) == chart.interaction.hovered)
        })
    });
    chart.interaction.animate_hover(target, elapsed_seconds, 0.16);
    renderer.prepare(chart, logical_size, scale)
}
```

호스트가 애니메이션 중 프레임을 요청해야 한다. 포인터가 멈춰도 진입·이탈이 끝날 때까지
진행하며, 렌더러 자체는 타이머를 돌리지 않는다. 클릭 선택은
`chart.interaction.selected = displayed.hit_test(position)`으로 설정하고 `None`으로 해제한다.
새 프레임을 그린 command buffer를 제출하고 이전 프레임을 놓은 뒤 `end_frame()`을 호출한다.
크기·DPR·내용 변경도 같은 `prepare` 경로를 사용하며, 단순 UI 편집 때 `clear_cache()`를
호출하면 재사용 가능한 라벨까지 버리므로 필요하지 않다.

### 저장과 호환성

`model`의 `serde` 기능을 켜면 모델을 직렬화할 수 있다. 새 조각별 옵션과 표기·상호작용 필드는
이전 스냅샷에 없을 때 기본값으로 복원한다. `RadialStyle`의 빠진 필드도 기본값을 사용한다.
저장된 모델과 복원한 모델은 모두 `validate`로 확인한다. 일시적인 호버·선택을 저장하지
않으려면 저장용 복사본의 `interaction`을 초기화한다.

JSON의 누락 필드 호환과 Rust 소스 호환은 다르다. 이전 `RadialSlice` 구조체 리터럴은 새 필드가
필요하므로 `RadialSlice::new`를 사용하는 편이 좋다. 차트·스타일은 `..Default::default()`나
기존 값의 복사본을 사용한다. 새 재질을 추가했으므로 재질 enum을 빠짐없이 분기하는 호스트도
분기를 갱신해야 한다. 이 원형 차트 API는 아직 개발 중이며 공개 릴리스에 포함하지 않았다.

## 네이티브 편집 예제

```bash
cargo run -p figgy-renderer --example radial_editor --features egui_demo
```

조각을 클릭하거나 왼쪽 목록에서 선택한 뒤 색, 개별 스타일, 표기 내용을 바꿀 수 있다.
`Whole chart`는 공통 설정을, `Individual style`은 선택한 조각의 설정을 편집한다.
갤러리와 편집기의 입체 시점은 시안에 맞춘 55°다. 0°는 정수리에서 내려다보는 시점이며,
값을 높이면 옆면이 더 드러난다. 정지 화면과 호버는 같은 시점을 사용한다.
`Inherit chart labels`는 개별 라벨 설정을 지운다. 프레임은 창 크기와 DPR에 맞춰 GPU 텍스처에
직접 그리며, 편집 화면에 보여 주기 위한 PNG 인코딩이나 CPU readback은 하지 않는다.
웹 Studio 연결은 별도 작업이다.

## 자원 수명과 검증

기본 GPU 예산은 256 MiB이며 `set_memory_budget`으로 바꾼다. 프레임마다 112바이트 uniform,
보이는 조각당 160바이트 표를 사용한다. 라벨 배치가 바뀌면 실제 출력 크기의 RGBA 라벨
텍스처를 만들고, 배치가 같으면 공유한다. 내보내기는 출력
타깃과 정렬된 readback 버퍼를 추가한다. GPU 할당 전에 전체 사용량과 예산을 검사한다.
`gpu_memory_usage()`는 살아 있는 자원과 GPU 완료를 기다리는 자원을 모두 집계한다.
기존 `Renderer`와 원장은 별도이므로 두 렌더러를 함께 쓰는 호스트는 두 사용량을 합산해 관리한다.

불변 프레임마다 작은 uniform·조각 표는 새로 만들 수 있으므로, 호스트는 사용이 끝난
프레임을 놓고 제출 뒤 `end_frame()`을 호출해야 한다. 호버할 때 라벨 해상도 크기의 자원이
계속 늘어나지는 않는다.

모든 관련 command buffer를 제출하거나 버린 뒤 `end_frame()`을 호출한다. 네이티브 호스트는
장치 poll로 완료 콜백을 처리하고, 웹에서는 브라우저가 콜백을 처리한다. 불필요한 캐시는
`clear_cache()`로 놓는다. 외부에 보관한 `Arc<RadialFrame>`이 있으면 해당 자원은 계속 유지된다.

```bash
cargo test -p figgy-model --features serde
cargo test -p figgy-renderer --test radial_render -- --test-threads=1
cargo test -p figgy-renderer --doc
cargo run -p figgy-renderer --example radial_gallery -- target/radial-gallery
```

픽셀 테스트는 비율·도넛 구멍·단일 조각의 이음새·열한 재질·세부 원·SSOT 필드별 갱신·
변경 없는 재사용·글꼴 등록·예산 거절·자원 회수·RGBA/BGRA/sRGB의 straight alpha·PNG 디코딩을
검사한다. 네이티브 GPU를 요청하지 못하면 테스트가 실패하며, 통과로 처리하지 않는다.

추가 픽셀 검증은 모서리의 실제 면적 변화, 배율별 외곽선, 내부 라벨의 들림과 자원 공유,
선택과 GPU 픽셀의 일치, 개별 색·표기·재질 변경, 호버 종료 시 복원, 반복 호버의 자원 상한을
검사한다. 모델 테스트는 직렬화 왕복과 기존 스냅샷의 기본값 복원도 검사한다.
종이 재질은 무늬를 끈 화면과의 픽셀 차이를 비교해, 옆면 높이 방향에도 결이 변하는지 검사한다.
이를 통해 종이 결이 세로줄로 늘어나는 회귀를 잡는다.
