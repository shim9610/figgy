# Recorded backend results

**Current support policy: macOS / Metal is experimental and not yet stable.**
All regression checks remain enabled in the normal package matrix. The separate
[precompiled Metal shader probe](metal-precision-probe/README.md) is an opt-in
investigation, not a default renderer option. Standard CI uses software/virtual
adapters; physical Intel/AMD/NVIDIA/Apple driver coverage remains outstanding.

## Renderer 0.18.0: Metal stream/resident line difference (PR #34)

Metal stays experimental for this release. On the Apple Paravirtual adapter, a
Constellation line drawn by the stream path differs from the same line drawn
resident by one 8-bit code value at a few pixels. Linux/Vulkan (lavapipe) and
Windows/DX12 match bit for bit. The
[PR #34 diagnostics](https://github.com/shim9610/figgy/pull/34) found:

- The streamed result is the same for chunk sizes 1, 2 and 7, and the
  difference is never more than one code value.
- Across 60 sub-pixel offsets, point stars alone never differ, while a single
  line alone differs at 8 offsets (10 pixels). The test line doubles back on
  itself, so its own segments overlap; the differing pixels sit where segments
  overlap or join. Grid and backdrop do not matter.
- The pixel that failed `styled_stream_runtime_matches_resident_pixels_and_ignores_point_maps`
  appeared when the 0.18.0 default right margin moved the data area by 24 px.
  The difference itself predates that change; the earlier investigation below
  already recorded a separate resident/streamed composition difference on Metal.

The likely cause is how intermediate results are stored when one line is split
across render passes (one pass per stream chunk, against one pass for the
resident draw). This is not confirmed: a run that put the whole line in one
chunk was cancelled before it reported. Until it is investigated on Metal
hardware, the stream-against-resident comparisons in
`streaming_style_runtime_tests.rs` accept, on Metal only, at most one code value
per channel on at most 32 pixels. Every other backend, and every other
comparison in those tests, stays exact.

## Renderer 0.16.1, 0.17.0 and master

Since 2026-10-12, CI runs only for pull requests ([when CI runs](README.md#when-ci-runs));
pushes to master do not start it. The results for renderer 0.16.1 / model 0.10.0
(published 2026-10-13) and renderer 0.17.0 / model 0.11.0 (published 2026-10-16)
are in the
[package runs](https://github.com/shim9610/figgy/actions/workflows/ci.yml?query=event%3Apull_request)
of the pull requests that introduced their changes.

Results for the renderer 0.18.0 / model 0.12.0 / figgy 0.15.0 release candidate
on master are recorded here after its CI run.

## Renderer 0.16.0 bounded area editing (2026-10-11)

Renderer 0.16.0 / model 0.10.0 / figgy 0.13.0 add bounded plot-area selection,
dragging and all eight resize handles through the common traits. A shared
native/WebGPU regression exercises radial, categorical and boxplot frames at
1x/2x: immutable prior frames, cache invalidation/reuse, axis lengths, visible
handles, data-pixel picking, PNG decoding, serialization and automatic-layout
reset. Model tests cover all eight handle directions and atomic corner rejection.
Default radial gallery pixels are compared with the renderer 0.15.1 output.
The native editors demonstrate handle capture and rollback of infeasible edits.

The [package run](https://github.com/shim9610/figgy/actions/runs/38139809649) for
commit `5cfab58` passed on Linux/Vulkan, Windows/DX12 and macOS/Metal. Browser
validation is a separate, opt-in Chromium/SwiftShader check run outside CI; the
three-OS package matrix runs native tests.

## Renderer 0.15.1 radial surfaces and export (2026-10-11)

Three native regressions cover visible top/wall outline creases (pie/donut,
bevel, rim/separator isolation, width zero and 1×/2×), nonperiodic wood grain
that varies along wall height, and decoded PNGs retaining partial hover and
persistent click selection. The PNG check also runs as an explicit WebGPU test;
it compares selected geometry with full hover, verifies physical lift and
checks that clearing selection restores the original pixels. The native package
matrix runs these checks on Linux/Vulkan, Windows/DX12 and macOS/Metal.

Local Linux/Mesa lavapipe checks pass, and the
[package run](https://github.com/shim9610/figgy/actions/runs/38136640766) for commit
`d20407d` passed on all three operating systems. The separate, opt-in browser check
used Chromium 151/SwiftShader; standard CI does not execute browser tests. To export
the hover and selection shown on screen, a host passes the same
`RadialChart.interaction` it used for display; see
[the export workflow](../crates/renderer/RADIAL.md#ssot-편집과-선택).

## Renderer 0.15.0 bounded chart interaction (2026-10-11)

This source release pairs renderer 0.15.0 with model 0.9.0 and figgy 0.12.0.
The [interaction guide](../crates/renderer/INTERACTION.md) covers renderer-owned
hit geometry, the shared selection/drag contracts, SSOT offsets and migration.
Boxplot and categorical chrome now share decoration drawing; their bounded data
models remain separate from Cartesian streaming. Existing Cartesian interactions
and saved flat radial styles retain their contracts.

The three-OS package matrix includes four new native interaction regressions:
radial outside/inside labels, boxplot axes/legend, and categorical data labels.
They check actual pixels after edits, selection, logical coordinates at different
scales, serialization, immutable old frames, data geometry and cache reuse.
The [package run](https://github.com/shim9610/figgy/actions/runs/38132858441) for
commit `d8d01a0` passed on all three operating systems.

Separate local validation passed the four native tests on Linux/Mesa lavapipe
and one Chromium 151.0.7922.173/SwiftShader browser test covering all three
renderers' interaction and image-export paths. Standard CI does not run that
browser test.

## Renderer 0.14.1 Windows initialization workaround (2026-10-11)

Windows DX12 uses explicit per-invocation zeroing and a barrier for six shared-array
compute entries. Other backends retain automatic initialization. The implementation
preserves the zero-value contract instead of globally disabling it. The
[diagnostic comparison](https://github.com/shim9610/figgy/actions/runs/38123340767)
identified the aggregate initialization DXIL as the trigger for WARP's first-use
JIT stall; see [scope and regression checks](shader-compilation.md#windows-dx12-workgroup-initialization).
That diagnostic run is not a package-matrix result; the
[package run](https://github.com/shim9610/figgy/actions/runs/38126175001) for commit
`4a1430c` passed on all three operating systems.

## Renderer 0.14.0 integration (2026-10-11)

The [master branch package runs](https://github.com/shim9610/figgy/actions/workflows/ci.yml?query=branch%3Amaster)
for the 0.14.0 revisions were cancelled before completing; the renderer 0.14.1
[package run](https://github.com/shim9610/figgy/actions/runs/38126175001) is the
first completed run that includes them.

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
A precompiled-shader experiment then investigated avoiding that dependency patch.
Its [first run](https://github.com/shim9610/figgy/actions/runs/38055501914) on Apple
Paravirtual Metal evaluated 768 projected coordinates per path: default WGSL and
the fast-math binary each failed 512 points; the precise binary passed all 768
with zero measured error. This was the extracted coordinate helper harness,
not the full picker or renderer. See the probe's recorded scope and build settings.
Renderer 0.14.0 needs neither the patch nor precompiled shaders: its `fma`
formulation passes on unmodified wgpu (see the 0.14.0 record above).

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

At this baseline, the causes had not yet been established; see the follow-up above.
At that revision the checks retained their original tolerances, and the source
release was not verified across all three backends. The 0.14.0 changes above record
the subsequent fixes and the specific pixel-comparison changes.

The first run stopped before the independent renderer integration-test binaries on
Metal. Follow-up runs execute them after a failure and use the shared
backend/fallback settings in all standalone native GPU fixtures. The package runs
recorded above for renderer 0.14.1 and later include those suites.

## Reproduce

Run the [package verification commands](README.md#run-locally) on the matching OS.
To rerun the three tests that failed on Metal at the 0.13.0 baseline (addressed in
0.14.0) in a checkout:

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

0.18.0에서도 Metal은 시험 단계다. Apple Paravirtual 어댑터에서 Constellation 선을
스트리밍으로 그린 결과와 상주로 그린 결과가 일부 픽셀에서 8비트 한 단계 다르다.
Linux/Vulkan과 Windows/DX12는 비트 단위로 같다. 점 별만 그리면 차이가 없고, 선 하나만
그려도 자기 구간이 겹치는 자리에서 차이가 난다. 렌더 패스를 나눠 그릴 때 중간값을 저장하는
방식이 원인으로 보이지만 확정하지 못했다. Metal 장비에서 조사하기 전까지
`streaming_style_runtime_tests.rs`의 스트리밍·상주 비교는 Metal에서만 채널당 한 단계,
최대 32픽셀까지 허용한다. 다른 백엔드와 다른 비교는 그대로 정확히 일치해야 한다.

0.16.0은 원형·범주형 막대·박스플롯 차트의 그리기 영역 선택·이동·크기 조정 검사를,
0.15.1은 원형 차트 표면과 PNG 내보내기 검사를, 0.15.0은 제목·축·라벨·범례의 선택과
이동 검사를 추가했다. 0.14.1은 Windows DX12의 작업 그룹 초기화 지연을 피했고, 0.14.0은
Windows의 DXC 선택 오류와 Metal의 좌표 반올림·중점 피킹 문제를 수정했다.
투명 픽셀은 검정·흰색 배경에 합성한 색과 알파를 각각 비교하며, 등고선 최적화 전후는
RGBA 채널마다 8비트 한 단계 이내여야 통과한다. 실패를 감추려고 검사를 건너뛰지 않았다.
로컬 렌더러 735개 검사와 실제 브라우저 WASM 검사 6개, 래퍼 수명 검사도 통과했다.

위 표와 실패 목록은 0.13.0 당시 기록이다. 2026-10-12 이전에 master에 올라간 리비전의
세 OS 결과는 [master CI](https://github.com/shim9610/figgy/actions/workflows/ci.yml?query=branch%3Amaster)에서
확인한다. 이후 리비전은 master push로 CI가 실행되지 않으므로 해당 리비전을 병합한
[PR의 CI 실행](https://github.com/shim9610/figgy/actions/workflows/ci.yml?query=event%3Apull_request)에서
확인한다. macOS는 계속 실험 단계로 표시하며, 실제 Apple GPU 검증은 별도로 남아 있다.
