# Shader compilation optimization

The public renderer API, four contour projection corrections, field interpolation,
hi/lo coordinate handling, invalid-value rejection and anti-aliasing rules are
unchanged. This change reorganizes compilation and dispatch, not chart quality.

## What changed

- Contour projection previously nested field lookup and numerical checks inside a
  correction loop. Software Vulkan compilers took disproportionately long to
  compile this entry. The new sequence seeds candidates, dispatches the same
  correction pipeline four times, finalizes candidates, then selects labels.
  Dispatch boundaries order the existing candidate buffer's reads and writes.
  Its coordinate lane temporarily stores the intermediate position: no additional
  working buffer or bind group is allocated. The two new pipelines are cached.
  There are five additional
  dispatches per automatic placement, with the same five field samples per candidate.
- WASM async prewarm now uses descriptors generated from the real Rust constructors.
  An auto-layout pipeline did not warm the subsequently created explicit-layout
  pipeline. Binding sizes/visibility, vertex inputs, blend state and sample count
  now match. The generated metadata is checked by a native regression test and
  never retains GPU objects. Ownership stays with wgpu. Existing initialization
  event names/order remain unchanged; the projection stage groups the three
  new compilation entries.
- Public single-pipeline helpers load their own module, rather than loading all
  six Cartesian shader modules. `Renderer` still creates the modules it needs.
- All production module/layout/pipeline creation goes through private inline
  wrappers. Native test builds capture the actual descriptors and durations;
  normal builds call wgpu directly. Coverage fails on a missing WGSL file or entry.

The automatic placement shader from before this change is frozen under
`crates/renderer/tests/fixtures/`. Same-backend image comparisons cover nonlinear
fields, panning, inverted/log axes, large coordinates, grid edges, transposed matrices,
NaN/infinity and zero gradients. Existing shader SSOT and renderer tests remain enabled.

## Measured in the Linux cloud environment

Measurements on 2026-10-10 used Rust 1.99, wgpu/Naga 30.0.1 and Chromium
151.0.7922.173. They are individual local runs, not physical-GPU or cross-OS results.

| Chromium / SwiftShader operation | Before | After |
| --- | ---: | ---: |
| Contour projection pipeline compilation | 32,921 ms | Seed 21.5 ms + correction 542.3 ms + final projection 578.9 ms |
| Sum of first pipeline creations in the coverage run | 45,382 ms | 13,981 ms |
| Sum of repeated pipeline creations | 893 ms | 868 ms |

The correction pipeline is compiled once, then dispatched four times. The coverage
run creates Cartesian renderers at sample counts 1 and 4; its totals include both
constructor sequences and other chart/streaming pipelines. This is not time to
first frame. Before: 21 unique sources, 129 pipeline creations. After: 21 sources,
133 creations, including the new seed/correction entries in each sequence. Repeated
creations above deliberately exercise the browser compiler cache; normal repeated
`prewarm_all` creates no shaders or pipelines at all.

The slowest remaining browser entries were field picking (2,319 ms), initial extent
reduction (1,865 ms) and contour anchor selection (1,433 ms). All 133 creations passed
the 20,000 ms budget used for this comparison. Fast entries were left intact.

In native SwiftShader (Subzero), the earlier projection-only investigation was
stopped after 166.7 seconds with about 14.4 GiB process RSS and no completed pipeline.
The optimized **whole coverage test** completed in 23.25 seconds with 433.4 MiB peak
process RSS; all entries passed the 20-second budget. These are process-memory
observations, not VRAM allocation counts or equivalent whole-run before/after totals.
Mesa lavapipe also passes the native compilation coverage test.

An actual Chromium WASM test separately passed full renderer prewarm, zero new
creation calls on repeated prewarm, automatic contour/label pixel checks and PNG
decode/round-trip validation. The wrapper's exact startup-event test also passed.
See [reproduction commands](README.md#shader-compilation-regressions).

Native regression validation on lavapipe covered 567 unit tests and 165 integration
tests; three existing opt-in probes remained ignored. The expanded projection
comparison initially used scalar upload for its large-coordinate fixture, losing
the intended fractional coordinates. After correcting that fixture to explicit
hi/lo upload, all ten old/new image comparisons passed in a targeted rerun. The
other 731 tests had passed in the full run. Eight renderer doctests, eight repository
audits, the native all-features/all-targets workspace check and the two actual WASM
tests also passed. New fixture and generated metadata files were confirmed present
in Cargo's package file list; the complete extracted-package matrix was not rerun
on Windows/macOS for this change.

The compiler behavior is consistent with excessive optimization work around nested
field searches and the enclosing correction loop. No specific compiler pass was
profiled, and this is not evidence of the same pathology on physical GPUs. The
extra dispatch boundaries may affect steady-state label-placement time differently
on different drivers. Windows and macOS must run their normal matrix; existing
Metal precision issues are not resolved by this optimization.

## 한국어 요약

등고선 위치 계산은 기존과 똑같이 네 번 보정한다. 다만 복잡한 필드 검색을 감싼 반복문을
셰이더 밖으로 옮겨, 같은 보정 파이프라인을 네 번 실행한다. 중간값은 기존 후보 버퍼에
저장하므로 GPU 버퍼가 추가되지 않는다. 최종 검증과 라벨 선택도 그대로 수행한다.

브라우저에서는 예열과 실제 생성의 레이아웃을 맞춰 중복 컴파일을 줄였다. 단일 파이프라인
생성 함수가 관계없는 셰이더까지 읽던 부분도 제거했다. 컴파일 전용 검사는 모든 셰이더와
진입점을 확인하며, 기존 렌더링·픽셀 검사는 별도로 유지한다. 위 수치는 이 Linux 소프트웨어
GPU 환경에서 얻은 결과다. 실제 GPU의 실행 속도나 macOS 정밀도 개선을 보장하는 수치는 아니다.
