# Recorded backend results

Recorded on 2026-10-10 at 10:20 UTC for public source
[`69ada34`](https://github.com/shim9610/figgy/commit/69ada34b81bc268d9ea7fda6cce9a3d1a7182d99):
figgy-renderer 0.13.0, figgy-model 0.8.0, figgy 0.11.0, Rust 1.99 and wgpu 30.
See the [Actions run](https://github.com/shim9610/figgy/actions/runs/38043202249)
for complete logs and diagnostic images. These are observations for that revision,
not a claim that every backend passes or that physical GPUs were tested.

| Standard runner | Actual rendering adapter | Build / WASM / API docs | Extracted package tests |
| --- | --- | --- | --- |
| Ubuntu 24.04 / Vulkan | llvmpipe, LLVM 20.1.2, Mesa 25.2.8 | Passed | Passed; 954 distinct tests, 3 ignored probes; GPU smoke repeated separately; 6 expected initialization failures verified |
| Windows 2025 / DX12 | See run log | Passed | Initial run still pending at the time of this record |
| macOS 15 ARM64 / Metal | Apple Paravirtual device | Passed | Failed; model 216 passed; renderer unit tests 558 passed, 7 failed, 1 ignored |

## Known Metal failures

The Metal adapter and device were created successfully. These tests failed after
initialization:

- `gpu_pick::tests::exact_gpu_f64_residual_selects_point_index`: no hit was returned
  for a point near a large f64 epoch with a fractional offset.
- `gpu_pick::tests::exact_gpu_line_boundary_endpoint_matches_cpu_exhaustive_picker`:
  an endpoint tie selected index 64 on the GPU instead of the CPU reference's 63.
- `renderer::streaming_runtime::selection::tests::stream_selection_and_data_match_resident_at_exact_display_scale`:
  at display scale 0.5, the largest channel difference was 5; the allowed difference
  is 3. Four later tests failed after this panic poisoned their shared font lock.

The cause of each independent failure has not yet been established. This evidence
does not establish that physical Apple GPUs or all Metal devices behave identically.
The checks remain enabled and their tolerances are unchanged. Until these failures
are resolved, this source release is not verified across all three backends.

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

공개 기본 러너에서 세 OS의 빌드·WASM 컴파일·API 문서 검사는 통과했다.
Linux의 패키지·렌더링·픽셀 검사도 통과했다. Windows의 전체 테스트는 이 기록을
작성할 당시 실행 중이었다. 최종 결과는 위 Actions 링크에서 확인할 수 있다.

macOS의 가상 Metal 장치에서는 정밀 피킹 2개와 스트리밍 픽셀 비교 1개가 실패했다.
나머지 4개 실패는 앞선 패닉으로 공유 잠금이 오염돼 발생했다. 테스트를 생략하거나
허용 오차를 늘리지 않았으며, 세 백엔드 검증이 완료됐다고 표시하지 않는다.
실제 Apple GPU에서도 같은 문제가 발생하는지는 별도 확인이 필요하다.
