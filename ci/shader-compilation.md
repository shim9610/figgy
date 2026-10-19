# Shader compilation optimization

Since renderer 0.13.1, contour label projection runs as separate GPU dispatches
so that software shader compilers finish quickly. Label candidates still receive
four projection corrections with the same field interpolation, hi/lo coordinate
handling, invalid-value rejection and anti-aliasing rules. The split changes
compilation and dispatch, not the public renderer API or chart quality.

## Compilation and dispatch design

- Contour projection is a dispatch sequence: seed candidates, run the same
  correction pipeline four times, finalize candidates, then select labels.
  Renderer 0.13.0 nested field lookup and numerical checks inside a correction
  loop in one entry, which software Vulkan compilers took disproportionately long
  to compile. Dispatch boundaries order the reads and writes of the existing
  candidate buffer. Its coordinate lane temporarily stores the intermediate
  position, so no additional working buffer or bind group is allocated. The seed
  and correction pipelines are cached. Automatic placement uses five more
  dispatches than the single-entry version, with the same five field samples per
  candidate.
- The single-invocation contour selector uses private scratch arrays instead of
  workgroup memory. A Windows WARP comparison measured first GPU completion at
  about 85 seconds with workgroup arrays and 0.23 seconds with private arrays;
  repeated dispatches were fast with both. Private arrays keep WGSL's
  initialization guarantees and the label selection order and capacity, and avoid
  WARP's expensive workgroup initialization.
- WASM async prewarm uses descriptors generated from the real Rust constructors,
  so binding sizes and visibility, vertex inputs, blend state and sample count
  match the pipelines the renderer creates later. A pipeline prewarmed with an
  automatic layout does not warm a later pipeline with an explicit layout. A
  native regression test checks the generated metadata, which never retains GPU
  objects; ownership stays with wgpu. Initialization event names and order are
  the same as in renderer 0.13.0; the projection stage groups the seed,
  correction and final projection entries.
- Public single-pipeline helpers load only their own module, not all eight
  Cartesian shader modules (six WGSL files and the two assembled vector modules).
  `Renderer` creates the modules it needs.
- All production module/layout/pipeline creation goes through private inline
  wrappers. Native test builds capture the actual descriptors and durations;
  normal builds call wgpu directly. Coverage fails on a missing WGSL file or entry.

## Windows DX12 workgroup initialization

Renderer 0.14.1 uses an internal, DX12-only override on Windows for six compute
entries: `reduce_values`, `reduce_states`, `pick_exact_candidates`,
`pick_reduce_candidates`, `pick_histogram_bin` and `scan_block`. Each invocation
zeros its own shared-array element and all invocations synchronize before any
calculation. Only these pipelines disable the now-redundant automatic zeroing.
Every field, including padding, is initialized. The two small replay-mask atomics
retain automatic initialization. Vulkan, Metal and browser WebGPU keep their
existing initialization policy; no dependency fork or shader passthrough is used.

The trigger was identified using actual Naga HLSL, actual DXC DXIL and symbolized
WARP ETW stacks. Naga's single-invocation aggregate zero becomes 768 or 512 static
stores in DXIL. WARP 1.0.20 spent about 97% of first-completion CPU samples in its
use/dependency/liveness analyses. A same-invocation loop with the same initialization
and barrier reduced extent completion from 17.61s to 0.166s; distributed element
initialization took 0.175s. These are individual Windows software-adapter diagnostic
measurements, not physical-GPU throughput claims. See the
[causal comparison run](https://github.com/shim9610/figgy/actions/runs/38123340767).

Regression tests inspect the WGSL entry ordering and exercise the actual production
initializers after poisoning every field. They check cross-invocation visibility
across four workgroups and repeated dispatches, including a 5-second DX12 budget
for creation plus first completion. Production extent reduction separately tests
1/63/64/65/127/128/129/4097 inputs, exact bounds and reuse, with a first-completion
budget. Pipeline capture checks that all six real constructors select the intended
policy. Browser metadata strips only the native workaround override and retains
WebGPU's normal automatic initialization. Rendering and pixel assertions remain
enabled on all three CI backends.

The old four-step automatic placement algorithm is frozen under
`crates/renderer/tests/fixtures/`. Its test loader replaces the marked shared
coordinate/sampling block with current SSOT math, so Metal coordinate fixes apply
to both algorithms. It also gives the two selection scratch arrays the same private
address space as production. The old projection loop and the dispatch sequence remain
independent. Same-backend image comparisons cover nonlinear fields, panning,
inverted/log axes, large coordinates uploaded as hi/lo, grid edges, transposed matrices,
NaN/infinity and zero gradients. Each output RGBA channel must differ by at most
one 8-bit code value: the old loop and separate dispatches can cross an AA rounding
threshold on Metal (45 pixels in the edge fixture, with identical alpha). This is
not a spatial tolerance; shifted or missing contours still fail. Existing shader
SSOT and renderer tests remain enabled.

## Measurements on Linux software GPUs

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
tests; three existing opt-in probes remained ignored. The old/new projection
comparison (ten image cases) passed in a separate run, and the other 731 tests
passed in the full run. Eight renderer doctests, the native all-features/all-targets
workspace check and the two WASM browser tests also passed. These local runs did
not include the extracted-package matrix on Windows or macOS.

The compiler behavior is consistent with excessive optimization work around nested
field searches and the enclosing correction loop. No specific compiler pass was
profiled, and this is not evidence of the same pathology on physical GPUs. The
extra dispatch boundaries may affect steady-state label-placement time differently
on different drivers; Windows and macOS results come from their normal package
matrix. The dispatch split does not change Metal precision. Renderer 0.14.0 fixes
coordinate-rounding boundaries and midpoint picking in shared WGSL; see
[backend results and fixes](STATUS.md).

## 한국어 요약

등고선 위치 계산은 기존과 똑같이 네 번 보정한다. 다만 복잡한 필드 검색을 감싼 반복문을
셰이더 밖으로 옮겨, 같은 보정 파이프라인을 네 번 실행한다. 중간값은 기존 후보 버퍼에
저장하므로 GPU 버퍼를 새로 만들지 않는다. 최종 검증과 라벨 선택도 그대로 수행한다.

브라우저에서는 예열과 실제 생성의 레이아웃을 맞춰 중복 컴파일을 줄였다. 단일 파이프라인
생성 함수가 관계없는 셰이더까지 읽던 부분도 제거했다. 컴파일 전용 검사는 모든 셰이더와
진입점을 확인하며, 기존 렌더링·픽셀 검사는 별도로 유지한다. 위 수치는 이 Linux 소프트웨어
GPU 환경에서 얻은 결과다. 실제 GPU의 실행 속도나 macOS 정밀도 개선을 보장하는 수치는 아니다.
