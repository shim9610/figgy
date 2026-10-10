# Experimental precompiled Metal shaders

macOS / Metal support is **experimental and not yet stable**. This opt-in probe
investigates supplying precise `.metallib` shaders while retaining unmodified
wgpu 30.0.1 for devices, buffers, submission and readback. It is an independent,
unpublished Cargo tool, not a renderer feature or a second production backend.

## Scope and assertions

The generator extracts the current `PickQueryTransform`, `PickQueryParams` and
coordinate projection helpers directly from the production `gpu_pick.wgsl`.
It validates a small compute harness with Naga, then emits MSL 2.3 with an explicit
resource map. There is no hand-maintained second copy of the coordinate formula.

On macOS, the Apple compiler builds the **same MSL source** twice: `-ffast-math`
and `-fno-fast-math`. Both binaries enter wgpu through
`create_shader_module_passthrough`, on a device requested with
`PASSTHROUGH_SHADERS`. Default WGSL runs on that same device as a third comparison.
The test neither patches wgpu nor changes the default renderer's compiler options.

Each path evaluates 12 fixtures with 64 points each: zero and positive/negative
1.7e12 epochs, normal/reversed X axes, and 100×100 / 3840×2160 pixel-coordinate
transforms with panel offsets. These are coordinate computations, **not 4K image
rendering tests**. An independent f64 oracle checks both projected pixels and
normalized positions. The chosen binary fractions are exactly representable; the
0.001-pixel bound detects lost residuals and is not a new general picking tolerance.

The precise path must pass all fixtures. Baseline outcomes are recorded without
assuming that every Metal device must reproduce the original bug. A missing
compiler/adapter/feature, failed shader creation, map failure or bad precise result
fails the experiment. Existing renderer failures remain visible in the normal CI.

This does **not** test full picking/gates/reduction, line endpoint tie rules,
field interpolation, draw shaders, blending, streaming or PNG output. Those are
subsequent integration stages. Precise compilation does not turn f32 into f64 or
guarantee identical pixels across GPU vendors.

## Run

Requires Rust 1.99, Python 3, and on macOS the Apple Metal compiler from Xcode.
From the repository root:

```sh
python3 ci/metal_precision.py
```

The public repository has a separate `Experimental Metal shader precision`
workflow on standard `macos-15`. It can be dispatched manually and runs on
`experiments/metal-precision-*` pushes. Private repositories do not run that job.
The workflow installs Apple's Metal toolchain component if the selected Xcode
does not already provide it. No registry publication is performed.

Linux development checks are separate and cannot establish a Metal pass:

```sh
python3 ci/metal_precision.py --generate-only
python3 ci/metal_precision.py --vulkan-control
```

Artifacts under `target/ci-results/metal-precision/` contain generated WGSL/MSL,
entry point, compiled binaries, exact compiler arguments, shader hashes and
per-fixture numeric results. The workflow log records compiler and adapter details.
The independent Cargo lockfile pins the experiment's dependencies.

## Promotion conditions

First validate binding and precision behavior in this small harness. Then integrate
the actual picker, including gate/reduction and tie cases, followed by the matching
draw/field transforms so rendered and picked coordinates agree. Keep WGSL as the
source, generate binaries reproducibly, and validate resource layouts on each
wgpu/Naga update. Test supported macOS versions and physical Apple GPUs in addition
to the virtual CI adapter before changing the support status or making this default.

No production feature name, public API or package version is committed by this
experiment. Support for an externally supplied device will need an explicit
feature-negotiation contract before a renderer option is offered.

## 한국어

macOS / Metal은 아직 불안정한 실험 단계로 둔다. 이 도구는 현재 피킹 코드에서 좌표
계산 함수를 추출해 같은 셰이더를 일반 경로와 정밀 Metal 바이너리 경로로 비교한다.
버퍼와 명령 제출은 기존 wgpu를 그대로 쓴다. 기본 렌더러에는 적용하지 않는다.

작은 좌표 계산 실험이 통과하면 실제 피킹 전체, 그리기·필드 계산, 이미지 출력 순으로
검증 범위를 넓힌다. 기존 CI 실패는 숨기지 않는다. 이 실험의 성공과 macOS 전체 지원
완료는 구분하며, 실행한 장치와 수치 결과를 함께 기록한다.
