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

## Exact pick variant table (#363)

`crates/renderer/tests/support/pick_variants.csv` lists every exact pick pipeline a
chart can reach, one row per key: the module, the shader set, the `PICK_VARIANT`
bits by name, the marker shape (or `any`), the override constants the browser
compiles, the series families that produce the key, the first producing case and the
number of producing cases, and the measured SwiftShader compile time with its budget.

- **Rows.** The renderer's `pick_variant_table_matches_the_picker` test
  (`renderer/picking/variant_table_tests.rs`) builds every series family that
  reaches the point picker (xy markers, lines, error bars, stems and fills, radar,
  ternary markers, lines and regions, streamlines, point vectors and box plot points)
  over every combination of the options that feed the exact variant (marker shapes
  and style maps, line dashes, fills, cartesian and polar axes, every draw style).
  It registers each chart, prepares its pick as `pick_chart` does and reads the keys
  from the picker: `exact_variants`, and `stream_exact_variants` where every series
  of the chart may stream. A chart that does not need the full shader set lists its
  keys for both sets. Options that do not feed the variant (colours, sizes, step
  interpolation, size and colour maps, axis scales and breaks, subplots, overlays and
  line offsets) are held at one value. The test fails when the reachable keys differ
  from the file, when a row has no time or budget, or when the module hashes below
  are stale.
- **Hashes.** For each exact module and shader set (`marker-line/full`, …), the
  table's header records `module_hash`, the 64-bit FNV-1a hash of the module's
  assembled source, and `module_fragments`, the WGSL files it is assembled from.
  `ci/test_pick_variants.py`, which `python3 ci/verify.py static` runs, rehashes
  each module from its fragments (joined with one newline, as the assembly joins
  them) and checks every row's budget. A change to a fragment of an exact module
  fails the static stage, naming the modules and how many rows each has; a WGSL
  change outside the exact modules does not touch the table. The table test checks
  the recorded hashes and fragment lists against the renderer's assembly.
- **Runner.** `node ci/shader_compile.cjs --pick-variants <capture> <table>` finds
  each row's assembled module in the native capture by its `module_hash`, and
  compiles `pick_exact_candidates` with `createComputePipelineAsync`, the capture's
  explicit layout and the row's constants, on a fresh device per row (a fresh
  browser every 100 rows). It fails a row over its budget. `--update` writes the
  measured times and budgets back; `--missing` measures only rows without a time.
  `FIGGY_SHADER_COMPILE_TIMEOUT_MS` (default 90,000) bounds one row.
- **Budget rule.** `budget_ms = max(3 x measured_ms, 2000)`, rounded up to the next
  100 ms. Three times the measured time absorbs run-to-run and machine variance on a
  software adapter; the floor keeps short compiles from failing on noise.

When an exact module changed:

```sh
# 1. The capture the runner reads (see ci/README.md).
FIGGY_SHADER_COMPILE_REPORT="$PWD/target/ci-results/shader-compilation-native.json" \
  cargo test --locked -p figgy-renderer --lib \
  all_shader_compilation_times_and_entry_coverage -- --test-threads=1
# 2. The rows and hashes. A row keeps its time while its module's hash is
#    unchanged; the changed modules' rows lose theirs.
FIGGY_UPDATE_PICK_VARIANTS=1 cargo test -p figgy-renderer --lib \
  pick_variant_table_matches_the_picker
# 3. Measure the rows without a time: only the changed modules'. All 2,001 rows take
#    about 1.5 hours on a 4-core Linux container.
node ci/shader_compile.cjs --pick-variants \
  target/ci-results/shader-compilation-native.json \
  crates/renderer/tests/support/pick_variants.csv --update --missing
```

Steps 2 and 3 also apply when the table test reports new or removed variants: new
rows have no time yet. Without `--update` the runner checks every row against its
committed budget. Like the rest of this script it needs a WebGPU-capable Chromium
and is not part of the three-OS matrix: no CI job runs it. Run it whenever the
table or a module hash changes; the static check forces that whenever an exact
module's WGSL changes.

## Pick warm-up (#363)

A browser renderer that compiles its render pipelines in the background (the
`FiggyChart` path, `for_window_with_shader_set_async_observed`) also compiles the
pipelines of each chart's first pick before that pick, and puts each into the
picker's own cache the moment its compile resolves:

- **When.** On each screen frame, for each chart it draws, when the chart's config
  revision, series revision or shader set has moved since the chart was last warmed
  (a generation stamp on the chart's state). So registration and every edit that can
  change a pick key warm the chart again; an unchanged chart costs one comparison.
- **What.** Only the chart's own pipelines, never the variant table, each over every
  layer a pick can reach (the active layer and, from the base layer, each subplot):
  - the exact keys (`Renderer::for_each_exact_pick_key`, from the pick plan and each
    series' pick descriptor); the variant-table test checks, for every chart it
    lists, that these are exactly the keys the pick needs;
  - the typed data pick kernels the chart's series can dispatch
    (`Renderer::data_pick_warm_kernels`: histogram, field contour and cell, vector
    grid, category bar, box plot, range bar, violin, parallel, triangle mesh and
    value labels, the last for a series that draws value labels selected or not);
  - while the picker is not enabled, its gate and reduce pipelines (`pick_gate_x`,
    `pick_gate_y`, `pick_reduce_candidates`).
  A pipeline the picker's caches hold, or one already compiling, is not scheduled.
- **How.** On the renderer's existing `CompileQueue` (`createComputePipelineAsync`
  with the checked layout and, for an exact key, its override constants), ahead of
  the waiting render compiles, the data kernels first (the longest compiles). A
  chart's few pick compiles are issued at once, past the queue's in-flight limit,
  instead of waiting behind the render compiles that took the slots first. They stay
  out of `pipeline_status`, `series_ready`, `pipelines_ready` and
  `pipeline_failures`, which stay about drawing. A native renderer compiles nothing
  early.
- **Committed when they resolve.** Each compile's completion creates the wgpu
  pipeline into the picker's cache itself, without a frame: the browser serves that
  creation from the compile while the resolved object is alive, which the completion
  keeps until it has committed. The last of gate and reduce to resolve enables the
  picker; an exact key or data kernel that resolves first waits for it, and commits
  then. The cache is shared with the completions through three `PickCell`s, an
  `Rc<RefCell>` on wasm (see "Shared pick caches" in `crates/renderer/ARCHITECTURE.md`,
  which also describes the cache's insert, dedupe and release steps).
- **The pick reads the cache.** `enable_gpu_picking_async`,
  `compile_pick_pipelines_at` and `compile_data_pick_pipelines_at` wait for a
  scheduled compile (issuing it first if it still waits), then find its pipeline in
  the cache. Without one they compile as before, so a pick never depends on the
  warm-up having worked.
- **Shader-set switch.** When a chart admits the full shaders, a browser renderer
  with a queue does not re-create a ready picker's gate and reduce pipelines
  synchronously on the page's thread: the picker goes back to not enabled, the next
  frame's warm-up schedules the full set's and enables it when they resolve, and an
  enable before that waits for them. A wasm Rust host on such a renderer therefore
  calls `enable_gpu_picking_async` before picking after a shader-set switch, as
  `FiggyChart` does: a synchronous pick right after it reports `PickerDisabled`.
  Native renderers re-create it as before.

### First pick on a static screen

Headless Chromium 157 with SwiftShader (`BROWSER_ARGS` in [ci/README.md](README.md)),
4-core Linux container, from `crates/renderer/tests/pick_warmup_static_browser.rs`
(`PICK_STATIC` lines): a new renderer whose background render compiles run, the
playground scene registered and drawn once, no frame after that, then the first pick
at the plot's centre (enable, prepare, the exact and data compiles, both picks).
"Before" is the same test on integration `6c5d5d5` (#373 merged), one run; "after" is
this change, two runs:

| Scene | 0 s before | 0 s after | 8 s before | 8 s after | 20 s before | 20 s after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Radar (spider, polar, value labels) | 24.3 s | 8.6–8.8 s | 17.8 s | 0.29–0.63 s | 41.6 s | 54–62 ms |
| Split violins | 13.0 s | 2.0–2.2 s | 10.5 s | 5–7 ms | 10.3 s | 36–48 ms |
| Box plot | 6.9 s | 4.0–4.3 s | 4.9 s | 8–9 ms | 4.4 s | 36–51 ms |
| Parallel coordinates | 1.4 s | 0.86–0.90 s | 0.86 s | 4–5 ms | 0.84 s | 33–46 ms |
| Triangle mesh | 2.9 s | 1.9 s | 1.8 s | 8–17 ms | 1.6 s | 53–55 ms |

At 8 s and 20 s the pick compiles nothing of its own and creates nothing twice (the
test counts each pick-labelled `createComputePipelineAsync` call by the module it
compiles from). At 0 s it waits for the warm-up's compiles. The radar at 8 s is the
closest: its compiles (the full set's value label kernel and exact key, both run
concurrently) end about 8.3–8.7 s after registration. A second test picks a chart of
every data pick family right after its first frame and checks that the warm-up
compiled every family's kernel and that the pick compiled none.

### Data pick kernel compile times

Each kernel alone, from the native capture, `node ci/shader_compile.cjs` on a
manifest of the data pick pipelines (first compile; the repeat is a cache hit):

| Kernel | Basic before | Basic after | Full before | Full after |
| --- | ---: | ---: | ---: | ---: |
| histogram | 0.44 s | 0.41 s | 2.9 s | 2.7 s |
| field contour | 5.5 s | 5.2 s | 14.2 s | 13.2 s |
| field cell | 1.7 s | 1.6 s | 10.2 s | 8.8 s |
| vector grid | 0.21 s | 0.20 s | 0.99 s | 0.82 s |
| category bar | 0.57 s | 0.43 s | 2.1 s | 0.70 s |
| box plot | 1.1 s | 0.93 s | 6.4 s | 3.3 s |
| range bar | 0.80 s | 0.55 s | 6.0 s | 1.8 s |
| **violin** | 8.6 s | 0.50 s | **71.5 s** | 1.6 s |
| parallel | 0.24 s | 0.25 s | 0.52 s | 0.56 s |
| triangle mesh | 1.1 s | 0.93 s | 4.0 s | 3.6 s |
| value label | 2.6 s | 2.0 s | 12.7 s | 4.6 s |

The violin kernel inlined `data_to_ndc`, the full set's projection with its axis cuts,
probability, log and polar paths, twelve times per curve half: twice in `box_slot` per
orientation branch, twice in `box_along` and once per `violin_point` call site. The
fix, as for the exact picker, calls each once inside a short loop with its arguments
chosen by `select`:
- `box_slot`: the slot's two edges, either orientation, one projection;
- `box_along`: one projection;
- the violin's two curve ends: one `violin_point`;
- `category_bar`: the bar's two ends and the slot's two edges, eight copies to one;
- the range bar's two ends: one `box_along`;
- the value label's radar spoke ends: one projection;
- the value label's member search: one call site;
- the value label's box: one `box_slot` (`box_narrowed`).
Calls and arguments are the same, so values and pixels are the same: the box plot,
violin, range bar, category bar, value label, radar, released-document and pick/draw
agreement tests pass unchanged. The field contour and cell kernels (up to 13 s on the
full set) are not pathological in this sense and are left as they are; they are
warmed like every other kernel.

## 한국어 요약

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

`crates/renderer/tests/support/pick_variants.csv`에는 차트가 도달할 수 있는 모든 exact pick
파이프라인 키와 SwiftShader 컴파일 시간·예산이 있다(#363). 행은 실제 픽 경로에서 자동 생성되고,
exact pick 모듈의 WGSL이 바뀌면 `verify.py static`이 바뀐 모듈과 행 수를 알려 주며 실패하므로,
표를 다시 만들고 그 모듈의 행만 다시 측정한다(`--missing`). 다른 WGSL 변경은 표와 관계없다.

브라우저 렌더러는 화면에 그리는 차트의 exact pick 파이프라인을 첫 픽 전에 기존 컴파일 큐로
미리 컴파일한다(#363). 차트 자신의 키만 컴파일하고, 픽 경로는 그 컴파일을 기다린 뒤 예전처럼
컴파일을 요청한다. 첫 프레임 직후 레이더 차트의 첫 픽은 약 10~13초에서 약 7.4초로, 8초 뒤의
첫 픽은 약 15초에서 약 2.4초로 줄었다. 이제는 gate·reduce 파이프라인과 차트 시리즈가 쓰는
데이터 픽 커널(모든 계열)도 같은 큐에 미리 올린다. 컴파일이 끝나면 완료 처리 코드가 그 자리에서
피커 캐시에 파이프라인을 넣으므로 프레임이 없어도 된다. 화면을 그대로 둔 채 8초·20초 뒤에 첫 픽을
하면 레이더 0.29~0.63초·54~62ms, violin·box·parallel·삼각 메시 4~55ms였고, 픽 시점의 컴파일은
없었다. full 셰이더 세트의 violin 커널은 투영 함수 인라인 복사를 줄여 71.5초에서 1.6초로,
value label 커널은 12.7초에서 4.6초로 줄었다. 값과 픽셀은 같다.
