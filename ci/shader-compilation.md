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

## Exact point picker compilation (#363)

The exact pick pipeline (`pick_exact_candidates`, label `figgy GPU pick exact
candidate pipeline`) is created on a series' first pick (#299, #332). On
Chromium's SwiftShader its compile time tracked two things, found by stubbing
one helper at a time in the assembled WGSL:

- **The number of inlined copies of the projection and validity helpers.**
  Time grew about linearly with them; neither the 32-point loop nor its trip
  count was a multiplier (a one-iteration loop or a dynamic bound measured
  the same). The area fill alone projected up to ten `data_to_ndc` copies
  per piece (four corners, `axis_baseline_ndc` holding one per axis) and
  called `sample_valid` four times; the error bar, stem, category jitter and
  path arrows held their own copies. The `PICK_VARIANT` override does not
  remove a disabled path's code in SwiftShader: a radar compiles the error
  bar path it never runs.
- **Long short-circuit `&&` / `||` chains.** Each becomes nested branches.
  One variant (line module, basic set, dash + styled line + fill) took
  230.9 s, of which 230 s went away when `sample_valid` used `&` / `|`.

The fix keeps every helper call and argument and therefore every value:
the helpers that were inlined several times are called once inside a short
loop over the corners, ends, quads or axes (`area_corner_ndc`,
`area_segment_drawn`, `axis_baseline_ndc`, `stem_end_ndc`, the stroke's
ends, the error bar's quads, the category slot and the arrowhead
Jacobian), and the validity tests (`sample_valid`, `pick_is_finite`,
`pick_coordinate_is_valid`) use the non-short-circuit `&` / `|` on pure
operands. The shared fragments change for the draw entries as well, with
the same values. No pipeline, module, cache or variant is added, and the
exact pipelines stay lazy.

Measured on 2026-11-19 with Playwright 1.63.0's headless Chromium
157.0.8103.78 (`--enable-unsafe-webgpu`, SwiftShader adapter) on a 4-core
Linux container. Each row is one `createComputePipelineAsync` in a fresh
browser, with the descriptor and explicit layout from the native capture:

| Exact variant (module, set, `PICK_VARIANT`, shape) | Before | After |
| --- | ---: | ---: |
| Marker, basic, 0, fixed | 612 ms | 550 ms |
| Marker, full, all paths, any | 4,770 ms | 2,652 ms |
| Marker-line, basic, 0, fixed | 1,341 ms | 850 ms |
| **Marker-line, full, area + polar, fixed (radar)** | **9,807 ms** | **2,685 ms** |
| Marker-line, full, all paths, any | 13,294 ms | 5,424 ms |
| Line, basic, 0 | 784 ms | 582 ms |
| Line, full, area + polar (radar without markers) | 7,210 ms | 2,008 ms |
| Line, basic, dash + styled line + area | 230,923 ms | 799 ms |
| Line, full, all paths | 8,481 ms | 3,045 ms |
| Vector, full, all paths | 1,185 ms | 1,098 ms |
| Radar, 40 render pipelines compiling first | 16,331 ms | 8,129 ms |
| Radar, all 339 captured render pipelines compiling first | 148,136 ms | 149,577 ms |

In the last row the exact pipeline finished after every background render
pipeline in both runs: it waited in the browser's compile queue, so its
own size no longer decides the time. These are single local runs on a
software adapter, not physical-GPU results.

## Exact pick variant table (#363, removed in #381)

The table of every exact pick pipeline a chart could reach
(`tests/support/pick_variants.csv`), its checker `ci/test_pick_variants.py`
and the runner's `--pick-variants` mode were removed with the move of a
registered chart's pick to the nearest-datum kernel and its ink hit (#381):
one kernel per projection and shader set replaces the specialized variants a
registered chart's pick compiled. The exact query remains for streamed
displays (rule 8; #381 phase 4 took the typed data pick off the exact point
query); its modules are measured with every other module in the capture above.

## Exact pick warm-up (#363, removed in #381)

A browser renderer used to compile each drawn chart's exact pick keys on its
compile queue ahead of the first pick (`warm_exact_picks`, measured by the
removed `tests/pick_warmup_browser.rs`). With a registered chart's pick and
typed data pick on the nearest-datum kernel (#381), no registered chart picks
through an exact variant any more, so that warm-up and its test are removed.

Its replacement (#381 follow-up, #395) warms the pipelines the nearest-datum
pick needs instead (`warm_first_pick`). For each chart a screen frame draws,
once per config and series revision and shader set, it queues on the same
compile queue, ahead of the waiting draw compiles, what the picker has not
created: the gate and reduce pipelines while picking is off, the nearest-datum
kernel of the chart's projection (flat, or 3D for a 3D chart) and the typed
data pick kernels of its series' families. Later frames create each pipeline
in the picker's cache as its compile resolves. A pick that comes first, and
`enable_gpu_picking_async`, `compile_pick_pipelines_async` and
`compile_data_pick_pipelines_async`, wait for a scheduled compile instead of
starting their own. No exact variant is warmed, and a monotonic chart or a
chart with a streamed series is not warmed (rule 8). These compiles stay out
of the screen's counts, waits and failures. Tests:
`a_drawn_charts_first_pick_compiles_each_pipeline_once`
(`crates/web/tests/compile_queue_browser.rs`) and
`a_pick_compile_goes_first_and_stays_out_of_the_screen_counts`
(`crates/renderer/src/compile_schedule.rs`).

## Data pick kernel compile times (#363)

Each typed data pick kernel alone, in headless Chromium 157 with SwiftShader on a
4-core Linux container (`BROWSER_ARGS` as in [ci/README.md](README.md)): the native
capture's data pick pipelines (`FIGGY_SHADER_COMPILE_REPORT` as above, filtered to the
`* data pick pipeline` labels) through `node ci/shader_compile.cjs`, first compile,
one run each. "Before" is integration `bda9357`.

| Kernel | Basic before | Basic after | Full before | Full after |
| --- | ---: | ---: | ---: | ---: |
| histogram | 0.44 s | 0.35 s | 2.72 s | 2.46 s |
| field contour | 4.11 s | 4.04 s | 12.32 s | 12.86 s |
| field cell | 1.38 s | 1.52 s | 9.14 s | 9.43 s |
| vector grid | 0.20 s | 0.18 s | 0.87 s | 0.89 s |
| category bar | 0.56 s | 0.50 s | 1.99 s | 0.78 s |
| box plot | 1.08 s | 0.98 s | 5.57 s | 3.48 s |
| range bar | 0.59 s | 0.50 s | 5.02 s | 1.67 s |
| range link | 0.51 s | 0.39 s | 3.91 s | 1.33 s |
| **violin** | 5.83 s | 0.51 s | **53.13 s** | 1.58 s |
| parallel | 0.22 s | 0.20 s | 0.50 s | 0.43 s |
| triangle mesh | 0.97 s | 1.10 s | 3.52 s | 3.36 s |
| value label | 2.06 s | 2.03 s | 12.23 s | 3.95 s |

The violin kernel inlined `data_to_ndc`, the full set's projection with its axis cuts,
probability, log and polar paths, twelve times per curve half: twice in `box_slot` per
orientation branch, twice in `box_along` and once per `violin_point` call site. As
for the exact picker, each projecting helper now calls it once inside a short loop,
with its arguments and result component chosen by `select`:

- `box_slot` (the slot's two edges, either orientation) and `box_along`;
- `category_bar` (the bar's two ends and the slot's two edges: eight copies to one);
- the range bar's two ends;
- the violin pick's two curve ends;
- the value label's radar spoke ends and its member search (one call site);
- the value label's box, which takes the narrowed place from its own `box_slot`
  (`box_narrowed`) instead of a second one.

Calls and arguments are the same, so values and pixels are the same. The bar, box,
range bar, violin, value label, radar, released-document and pick/draw agreement tests
pass unchanged. Differences under about 0.3 s are run-to-run noise. The field contour
and cell kernels (up to 13 s on the full set) are not pathological in this sense and
are unchanged.

## 한국어 요약

데이터 픽 커널(#363) 중 violin·value label·category bar·range bar·box 커널은 투영 함수
`data_to_ndc`를 분기와 끝점마다 따로 인라인하던 것을 짧은 반복문 안의 한 번 호출로 바꿨다.
값과 픽셀은 같다. full 셰이더 세트의 SwiftShader 컴파일은 violin 커널이 53.1초에서 1.6초로,
value label 커널이 12.2초에서 4.0초로 줄었다.

등고선 위치 계산은 기존과 똑같이 네 번 보정한다. 다만 복잡한 필드 검색을 감싼 반복문을
셰이더 밖으로 옮겨, 같은 보정 파이프라인을 네 번 실행한다. 중간값은 기존 후보 버퍼에
저장하므로 GPU 버퍼를 새로 만들지 않는다. 최종 검증과 라벨 선택도 그대로 수행한다.

브라우저에서는 예열과 실제 생성의 레이아웃을 맞춰 중복 컴파일을 줄였다. 단일 파이프라인
생성 함수가 관계없는 셰이더까지 읽던 부분도 제거했다. 컴파일 전용 검사는 모든 셰이더와
진입점을 확인하며, 기존 렌더링·픽셀 검사는 별도로 유지한다. 위 수치는 이 Linux 소프트웨어
GPU 환경에서 얻은 결과다. 실제 GPU의 실행 속도나 macOS 정밀도 개선을 보장하는 수치는 아니다.

정확 점 선택(exact pick) 파이프라인은 같은 투영·유효성 함수를 여러 번 인라인하던 자리에서 짧은
반복문 안의 한 번 호출로 바꾸고, 긴 `&&` 체인을 `&`로 바꿨다(#363). 호출 함수와 인자가
같으므로 결과값은 그대로다. SwiftShader에서 레이더 변형은 9.8초에서 2.7초로, 가장 느리던
변형은 230.9초에서 0.8초로 줄었다. 렌더 파이프라인 339개가 먼저 큐에 있으면 대기 시간이
지배해 약 150초로 변하지 않는다.

차트가 도달할 수 있는 모든 exact pick 파이프라인 키를 담던 표(`pick_variants.csv`)와 그 검사기,
측정 모드(`--pick-variants`)는 #381에서 없앴다. 등록 차트의 픽이 최근접 데이터 커널과 잉크 적중으로
바뀌어, 투영과 셰이더 세트마다 커널 하나가 특수화된 변형들을 대신한다. exact 질의는 스트리밍
화면에만 남고(#381 4단계에서 typed 데이터 픽도 exact 점 질의를 더 쓰지 않는다), 그 모듈은 위의 캡처에서 함께 잰다.

브라우저 렌더러가 화면에 그리는 차트의 exact pick 파이프라인을 첫 픽 전에 미리 컴파일하던
워밍업(#363)과 그 측정 테스트는 #381에서 없앴다. 등록 차트의 픽과 typed 데이터 픽이 모두 최근접
데이터 커널을 쓰므로 exact 변형으로 픽하는 등록 차트가 더 없다.

그 자리는 최근접 데이터 픽에 필요한 파이프라인을 미리 컴파일하는 워밍업(`warm_first_pick`, #381
후속, #395)이 대신한다. 화면 프레임이 그리는 차트마다 설정·시리즈 리비전과 셰이더 세트가 바뀔 때
한 번, 피커가 아직 만들지 않은 파이프라인을 같은 컴파일 큐에서 대기 중인 그리기 컴파일보다 앞에
올린다. 피킹이 꺼져 있으면 gate·reduce 파이프라인을, 차트 투영에 맞는 최근접 데이터 커널(평면,
3D 차트면 3D)을, 시리즈 계열에 맞는 typed 데이터 픽 커널을 올린다. 이후 프레임은 컴파일이 끝나는
대로 각 파이프라인을 피커 캐시에 만든다. 그보다 먼저 온 픽과 `enable_gpu_picking_async`,
`compile_pick_pipelines_async`, `compile_data_pick_pipelines_async`는 따로 컴파일하지 않고 예약된
컴파일을 기다린다. exact 변형은 미리 컴파일하지 않으며 단조 차트와 스트리밍 시리즈가 있는 차트도
제외한다(규칙 8). 이 컴파일은 화면의 집계·대기·실패에 넣지 않는다. 테스트는
`a_drawn_charts_first_pick_compiles_each_pipeline_once`(`crates/web/tests/compile_queue_browser.rs`)와
`a_pick_compile_goes_first_and_stays_out_of_the_screen_counts`(`crates/renderer/src/compile_schedule.rs`)다.
