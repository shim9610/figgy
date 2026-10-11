# Recorded backend results

**Current support policy: macOS / Metal is experimental and not yet stable.**
All regression checks remain enabled in the normal package matrix. The separate
[precompiled Metal shader probe](metal-precision-probe/README.md) is an opt-in
investigation, not a default renderer option. Standard CI uses software/virtual
adapters; physical Intel/AMD/NVIDIA/Apple driver coverage remains outstanding.

## Renderer 0.14.0 integration (2026-10-11)

Use the [master branch package runs](https://github.com/shim9610/figgy/actions/workflows/ci.yml?query=branch%3Amaster)
for the merged revision's result. A configured job or a narrowly scoped experiment
is not a completed package-suite pass.

- **Windows compiler setup:** `WGPU_DX12_COMPILER` now receives `dxc`, not the DLL
  path. Pinned DXC DLLs are placed beside each extracted test executable. Previously
  wgpu ignored the invalid value and fell back to FXC, producing loop-unroll and
  internal compiler errors before some rendering checks could run.
- **Windows first contour dispatch:** the single-invocation label selector now
  uses private arrays instead of workgroup arrays. In the
  [WARP memory A/B probe](https://github.com/shim9610/figgy/actions/runs/38115192492),
  first GPU completion fell from about 85 seconds to 0.23 seconds; cache replay
  and exact 1,024-label checks passed. WGSL initialization guarantees and label
  selection order are unchanged. The 30-second deadline is unchanged, and test
  readback checks completion and mapping errors before accessing a buffer.
  Disabling the instance debug flag did not fix the delay: wgpu already avoids
  debug shaders on this WARP version. CI retains its default validation flags.
- **Metal coordinate precision:** explicit `fma` rounding boundaries preserve
  hi/lo residuals and field-grid subtraction. The [unmodified-wgpu experiment](https://github.com/shim9610/figgy/actions/runs/38109490393)
  passed all 768 projection cases with this formulation; the default expression
  failed 512. A [second probe](https://github.com/shim9610/figgy/actions/runs/38109681517)
  also covered a non-binary viewport/range ratio. This experiment isolates arithmetic;
  the full matrix additionally checks picking, contours and field interpolation.
- **Line midpoint picking:** CPU and GPU select the earlier endpoint within the
  projected f32 rounding uncertainty. Regressions check both sides of the midpoint.
- **Zero-distance line picking:** interior distance is measured perpendicular to
  the segment instead of reconstructing a rounded closest point. The
  [DX12/WARP A/B probe](https://github.com/shim9610/figgy/actions/runs/38111540388)
  measured a false `9.5367386e-7 px` distance for a cursor on a horizontal line in
  the old path, which missed at zero tolerance; the corrected path returns exactly
  zero and the expected hit. CPU and GPU use the same formula. Finite endpoints,
  near misses and unowned streaming halos remain covered; no hit tolerance increased.
- **Transparent pixel comparisons:** compare composited colors on both black and
  white, keeping the three-code-value visible-color limit and separately bounding
  alpha error to one. The old unpremultiplied-RGB comparison amplified subpixel
  rounding at low alpha. Counterexamples reject lost coverage and visible changes.
- **Contour projection oracle:** the old four-correction loop remains independent
  from the new dispatch sequence and shares current coordinate math. Every RGBA
  channel must stay within one 8-bit code value. The Metal edge fixture differed
  at 45 pixels by one color code value, with identical alpha; large vector dumps
  are replaced by a bounded diagnostic.

Local Linux/lavapipe validation passed 735 renderer tests, with three existing
opt-in probes ignored. Actual Chromium/SwiftShader execution passed six WASM tests
covering prewarm, startup events and contour/label pixels, plus the wrapper lifecycle
gate. These browser checks are separate from the three-OS native package matrix.

## Earlier investigation and baseline

Follow-up [compiler A/B diagnostics](https://github.com/shim9610/figgy/actions/runs/38046669897)
found that disabling Metal fast-math in a temporary wgpu-hal patch made both
precision-picking tests and the field-location integration test pass. The
resident/streamed pixel difference remained. This identifies a compiler-sensitive
precision issue and a separate composition issue; it does not verify all Apple GPUs.
The new precompiled-shader experiment investigates avoiding that dependency patch.
Its [first run](https://github.com/shim9610/figgy/actions/runs/38055501914) on Apple
Paravirtual Metal evaluated 768 projected coordinates per path: default WGSL and
the fast-math binary each failed 512 points; the precise binary passed all 768
with zero measured error. This was the extracted coordinate helper harness,
not the full picker or renderer. See the probe's recorded scope and build settings.

The table below is the original recorded baseline, not the later experiment.

Recorded on 2026-10-10 for public source
[`69ada34`](https://github.com/shim9610/figgy/commit/69ada34b81bc268d9ea7fda6cce9a3d1a7182d99):
figgy-renderer 0.13.0, figgy-model 0.8.0, figgy 0.11.0, Rust 1.99 and wgpu 30.
See the [Actions run](https://github.com/shim9610/figgy/actions/runs/38043202249)
for complete logs and diagnostic images. These are observations for that revision,
not a claim that every backend passes or that physical GPUs were tested.

| Standard runner | Actual rendering adapter | Build / WASM / API docs | Extracted package tests |
| --- | --- | --- | --- |
| Ubuntu 24.04 / Vulkan | llvmpipe, LLVM 20.1.2, Mesa 25.2.8 | Passed | Passed; 954 distinct tests, 3 ignored probes; GPU smoke repeated separately; 6 expected initialization failures verified |
| Windows 2025 / DX12 | Microsoft Basic Render Driver, WARP 1.0.20.0 | Passed | Model 216 passed; renderer tests reported failures before this run was superseded; no completed suite result |
| macOS 15 ARM64 / Metal | Apple Paravirtual device | Passed | Failed; model 216 passed; renderer unit tests 558 passed, 7 failed, 1 ignored |

## Windows observations

The WARP adapter, device creation and GPU submission smoke test passed. The renderer
unit-test binary reported failures, beginning with
`data_render::stream_point_style_tests::tests::streamed_style_point_and_errorbar_entries_match_resident_pixels`.
Later streaming and resident-renderer tests also reported failures. Picking tests
continued to make progress, so the long-running job was not simply stuck at startup.

The job was cancelled by a newer push before the unit-test binary completed. The
first workflow captured panic details until that binary finished, so its partial
log does not establish the causes or a final failure count. This is **not a passing
Windows result**. Follow-up runs stream test output immediately with `--nocapture`
and keep independent test binaries running after failures. Rendering checks and
their existing tolerances remain enabled on WARP.

## Metal failures at the 0.13.0 baseline

The Metal adapter and device were created successfully. These tests failed after
initialization:

- `gpu_pick::tests::exact_gpu_f64_residual_selects_point_index`: no hit was returned
  for a point near a large f64 epoch with a fractional offset.
- `gpu_pick::tests::exact_gpu_line_boundary_endpoint_matches_cpu_exhaustive_picker`:
  an endpoint tie selected index 64 on the GPU instead of the CPU reference's 63.
- `renderer::streaming_runtime::selection::tests::stream_selection_and_data_match_resident_at_exact_display_scale`:
  at display scale 0.5, the largest channel difference was 5; the allowed difference
  is 3. Four later tests failed after this panic poisoned their shared font lock.

At this baseline, the causes had not yet been established; see the follow-up above. This evidence
does not establish that physical Apple GPUs or all Metal devices behave identically.
At that revision the checks retained their original tolerances, and the source
release was not verified across all three backends. The 0.14.0 changes above record
the subsequent fixes and the specific pixel-comparison changes.

The first run stopped before the independent renderer integration-test binaries on
Metal. The CI follow-up uses `--no-fail-fast` and continues with doctests and negative
initialization checks, while retaining a failing final result. Follow-up runs also
use the shared backend/fallback settings in all standalone native GPU fixtures.
Consult the latest commit's run before drawing conclusions about those suites.

## Reproduce

Run the [package verification commands](README.md#run-locally) on the matching OS.
For an individual Metal failure in a checkout:

```sh
WGPU_BACKEND=metal cargo test --locked -p figgy-renderer --features serde --lib \
  gpu_pick::tests::exact_gpu_f64_residual_selects_point_index -- --exact --nocapture
WGPU_BACKEND=metal cargo test --locked -p figgy-renderer --features serde --lib \
  gpu_pick::tests::exact_gpu_line_boundary_endpoint_matches_cpu_exhaustive_picker -- --exact --nocapture
WGPU_BACKEND=metal cargo test --locked -p figgy-renderer --features serde --lib \
  renderer::streaming_runtime::selection::tests::stream_selection_and_data_match_resident_at_exact_display_scale \
  -- --exact --nocapture
```

Running the primary failures individually avoids obscuring their result with the
shared-lock failure cascade. The Actions package job additionally tests the actual
`.crate` contents outside the source checkout.

## 한국어 요약

0.14.0에서는 Windows의 DXC 선택 오류와 Metal의 좌표 반올림·중점 피킹 문제를 수정했다.
투명 픽셀은 검정·흰색 배경에 합성한 색과 알파를 각각 비교하며, 등고선 최적화 전후는
RGBA 채널마다 8비트 한 단계 이내여야 통과한다. 검사 생략으로 실패를 숨기지 않는다.
로컬 렌더러 735개 검사와 실제 브라우저 WASM 검사 6개, 래퍼 수명 검사도 통과했다.

위 표와 실패 목록은 0.13.0 당시 기록이다. 병합된 리비전의 세 OS 최종 결과는
[master CI](https://github.com/shim9610/figgy/actions/workflows/ci.yml?query=branch%3Amaster)에서
확인해야 한다. macOS는 계속 실험 단계로 표시하며, 실제 Apple GPU 검증은 별도로 남아 있다.
