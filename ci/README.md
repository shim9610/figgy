# Build and rendering verification

## When CI runs

The full three-OS matrix runs only for **public, non-draft pull requests** when
opened, updated, reopened or marked ready for review. Branch pushes without a PR,
master/merge pushes and tag pushes do not start it. A push to an open PR triggers
one PR run; a newer run cancels the older run for that PR. Draft PRs wait until
they are ready. Failed jobs can be rerun from that PR's Actions run. A master
commit has no run of its own: its results are in the last run of the pull
request that merged it.

CI sets `RUSTFLAGS` and `RUSTDOCFLAGS` to `-D warnings` in all three jobs, so a
compiler or rustdoc warning in the libraries, examples, tests or extracted registry
packages fails the job. Test-only input adapters and inspection helpers compile
only with `cfg(test)`; the normal native and WASM libraries do not carry those
fixtures.

Prepare changes on a public candidate branch and open a PR to request CI. All
three backend jobs run on every such PR, including experimental Metal. The
separate Metal precision probe is manual-only.

GitHub's standard hosted runner compute is free for public repositories; artifact
storage has its own limits.

The public GitHub Actions workflow uses **standard hosted runners**:

| Runner | Requested backend | Rendering device |
| --- | --- | --- |
| `ubuntu-24.04` | Vulkan | Mesa lavapipe when no render device is present |
| `windows-2025` | DX12 | Microsoft WARP software adapter |
| `macos-15` (ARM64) | Metal | The runner's exposed Metal adapter, normally virtualized |

No larger, dedicated GPU or self-hosted runner is required. These jobs cover
backend behavior; they do not certify physical Intel/AMD/NVIDIA/Apple GPU drivers.
The actual adapter and backend are printed in each run. A missing adapter, rejected
device or failed render **fails the job**; there is no successful CPU-only fallback
for the Metal job and no skipped GPU initialization.

macOS / Metal support is currently **experimental, not stable**. Its full matrix
job remains required; see the PR run that tested the revision and the
[fix history](STATUS.md).
The separate, opt-in
[Metal precision experiment](metal-precision-probe/README.md) compares default WGSL
with fast and precise precompiled Metal shaders without patching wgpu. Success in
that narrowly scoped probe is not a passing renderer/package matrix.

Windows CI selects `WGPU_DX12_COMPILER=dxc` and installs pinned DXC DLLs beside
the extracted test executables. This variable takes a compiler name, not a DLL path.
wgpu's default validation flags stay enabled, and Windows uses the same GPU
completion deadlines and pixel tolerances as the other backends.
[Shader compilation](shader-compilation.md#windows-dx12-workgroup-initialization)
covers the shader changes that avoid WARP's slow first-use workgroup initialization.

## Security analysis

The separate `CodeQL` workflow runs only for public, non-draft pull requests,
using the same opened/updated/reopened/ready-for-review events as the package
matrix. It has no push, scheduled or manual trigger. It analyzes Rust, Python,
JavaScript/TypeScript and GitHub Actions with the `security-extended` query suite
on standard Ubuntu runners. CodeQL actions are pinned to exact commit IDs.

The Rust analysis uses CodeQL's supported `none` build mode. This source analysis
does not run GPU tests, prove coverage of every feature/target configuration, or
analyze WGSL shaders. The native/WASM builds and renderer pixel regressions remain
in the package matrix. The other CodeQL languages also use analysis without a
project build. No application build script is explicitly run by this workflow.

Rust CodeQL can report incomplete procedural-macro expansion and internal type
inference diagnostics even when analysis/upload succeeds and the Rust compiler
checks pass. Inspect the SARIF diagnostics and extraction counts in the job log;
zero PR findings do not establish complete Rust coverage. Such
diagnostics are retained, and the compiler and GPU regressions remain independent
required checks.

The workflow uses `pull_request`, not `pull_request_target`, with read-only source
access and `security-events: write` for results. It requires no repository secret
and does not change source or merge a suggested fix. GitHub's PR code-scanning
results distinguish successful analysis from vulnerability findings; a green
analyzer job alone is not a claim that no alerts exist. Review the PR's findings
and the repository's configured severity thresholds. SARIF reports are retained
for seven days as `codeql-<language>` artifacts so results can also be inspected
without repository security-administration access.

There is no scan of every default-branch push or periodic rescan of unchanged
code under this PR-only policy. CodeQL reports a workflow warning recommending a
push trigger for default-branch alerts; this is expected with the chosen policy.
Use the PR findings view, rather than interpreting an empty default-branch alert
list as a clean scan. Dependabot alerts remain a separate source of
known dependency vulnerability reports. Report suspected vulnerabilities through
[the security policy](../SECURITY.md).

## What runs on every OS

Before compilation, `ci/verify.py static` runs the package-document regression
tests and checks package-local links using offline
`cargo package --list --exclude-lockfile`. Excluding generated lockfile work
avoids dependency resolution, including on runners with an empty Cargo cache;
normal release packaging still includes and validates `Cargo.lock`.
The tracked `.githooks/pre-push` runs the same checker against each outgoing
commit's archived source, so a repaired working tree cannot hide a broken branch
being pushed. See [hook installation](../CONTRIBUTING.md#build-and-test).
The extracted-archive audit reuses the same link rules after packaging.

1. Native workspace checks with default and all optional features, including examples.
2. Compilation for `wasm32-unknown-unknown`, including every browser test target
   (`cargo check --workspace --target wasm32-unknown-unknown --test '*_browser'`), and strict Rust API documentation links on
   native and WASM targets. The WASM check catches links to native-only methods.
   CI compiles the browser tests but does not run them; running them needs a
   WebGPU browser (see below).
3. Creation and Cargo verification of both `.crate` archives, without registry upload.
   Archive contents must include source resources, font/license notices, guides,
   examples and test fixtures. Gallery assets and unsupported integrations are
   excluded; relative guide links must resolve inside the extracted package.
   `package-audit.json` records exact sizes, SHA-256 values and file inventories.
4. Extraction outside the checkout, followed by model/renderer unit tests, integration
   tests and doctests with `serde` enabled. This includes native GPU pixel, SSOT edit,
   cache reuse, streaming, picking and memory regressions, plus radial, categorical,
   boxplot, radar, polar, vector-field, area/step-line, overlay-layer, subplot and
   annotation-layer rendering tests. Bounded interaction regressions select and drag
   renderer-measured titles, axes, labels and legends, verify changed pixels and
   unchanged data geometry, and check serialization, scale and cache reuse. The
   normal suite also requests an unsupported device limit to test the device-error
   path.
5. Three subprocess checks with no enabled adapters: each must run its intended test
   and fail on adapter initialization. Linux additionally exercises an unavailable
   Vulkan driver.

Pixel checks compare rendered contents and edited/cached output with a fresh render
on the same backend. They are not an assertion that all drivers produce byte-identical
images. Existing opt-in stress/probe tests remain `#[ignore]` and are reported by Rust.
Tests run sequentially so GPU memory measurements do not overlap. These are headless
native renderer tests; WASM compilation is not a browser execution test.

The docs.rs metadata enables `serde` without GUI demo dependencies. Renderer API
pages are built for native Linux and WASM; the model needs only the native default
target. These builds do not initialize a graphics adapter. docs.rs builds its
pages from each published crate version after publication, not from master.
Local rustdoc builds do not check what docs.rs hosts.

Logs and generated diagnostic PNGs are uploaded as `verification-<runner>-<backend>`
artifacts, including on failure, and retained for seven days. The logs identify which
checks actually completed. The configured matrix alone is not evidence of a pass;
consult the Actions run of the PR that tested the commit and the
[recorded backend results](STATUS.md).
Independent test binaries, doctests and missing-adapter checks continue after a test
failure, while the final job still fails. This prevents one failing suite from hiding
the rest of the backend results.
Test output is streamed immediately, so a cancellation or timeout does not hide
panic details until the end of a large test binary.

## Shader compilation regressions

The renderer unit suite measures the real shader and pipeline constructors with
`Instant`. It covers every shipped WGSL module and entry point, Cartesian pipelines
at sample counts 1 and 4, streaming/replay, picking/fit, and the radial, categorical,
boxplot and radar renderers. A missing source or entry fails coverage. Repeating
`prewarm_all` must create no shaders or pipelines; a single-pipeline builder must
load only its own shader. No timing instrumentation is included in native release
builds.

The package job saves `shader-compilation-native.json` in the
`verification-<runner>-<backend>` artifact.
Each individual module or pipeline must finish within 30,000 ms by default.
`FIGGY_SHADER_COMPILE_MAX_MS` can set a positive per-operation budget. This is a
regression ceiling, not a performance guarantee across hardware or a timeout that
can interrupt a blocked driver. The job's overall timeout still applies. Measurements
include validation and backend compilation; driver caches may already be warm.

To capture a standalone run (POSIX shell):

```sh
mkdir -p target/ci-results
FIGGY_SHADER_COMPILE_REPORT="$PWD/target/ci-results/shader-compilation-native.json" \
  cargo test --locked -p figgy-renderer --lib \
  all_shader_compilation_times_and_entry_coverage -- --nocapture --test-threads=1
```

`shader_pipeline_contract.json` contains generated descriptor metadata, without
WGSL source. WASM async prewarm uses the actual explicit layouts, vertex attributes
and blend state from this file. The unit test compares it with live Rust constructor
descriptors. After changing a shader or descriptor, regenerate it once with
`FIGGY_UPDATE_SHADER_CONTRACT=1` on the command above, then rerun **without** that
flag. Never set the regeneration flag in CI.

Browser measurements are a separate, opt-in check; the three-OS matrix does not
claim to execute a browser. Install Playwright in a temporary tools directory and
place its `node_modules` on `NODE_PATH`, or use an existing Playwright installation.
For example:

```sh
npm install --prefix /tmp/figgy-browser-tools --no-save --package-lock=false playwright@1.62.1
export NODE_PATH=/tmp/figgy-browser-tools/node_modules
# Supply an installed WebGPU-capable Chromium, or install Playwright's browser.
export BROWSER_EXECUTABLE_PATH=/path/to/chromium
node ci/shader_compile.cjs \
  target/ci-results/shader-compilation-native.json \
  target/ci-results/shader-compilation-browser.json
```

This script compiles every captured descriptor, then repeats each creation to
measure cache reuse. Shader errors, device loss, missing measurements, budget
overruns and operation timeouts fail the check. Partial measurements survive a
failure. `FIGGY_SHADER_COMPILE_TIMEOUT_MS` defaults to 90,000 ms. Driver selection
is left to the browser/environment; SwiftShader is not forced on hardware machines.

For actual renderer initialization and PNG pixels, build and run the WASM test:

```sh
cargo test --locked -p figgy-renderer --target wasm32-unknown-unknown \
  --test shader_prewarm_browser --no-run --message-format=json > /tmp/figgy-wasm-build.jsonl
# The runner must match Cargo.lock's wasm-bindgen version.
export WASM_BINDGEN_TEST_RUNNER=/path/to/wasm-bindgen-test-runner
node ci/wasm_browser_test.cjs /tmp/figgy-wasm-build.jsonl
```

On a machine without a GPU, headless Chromium has a SwiftShader WebGPU adapter but
cannot create a WebGPU canvas swap chain with its default flags: chart creation then
fails with "A valid external Instance reference no longer exists". Pass the Vulkan
SwiftShader flags through `BROWSER_ARGS`:

```sh
export BROWSER_ARGS='--enable-features=Vulkan --use-angle=vulkan --enable-unsafe-swiftshader'
```

The test runs real window initialization and full prewarm, checks that a second
prewarm creates nothing, renders automatic contour labels, and decodes the exported
PNG to check its pixels. Change `--test` to run another renderer browser test the
same way, for example `bounded_interaction_browser` or `radar_browser`. The `figgy`
wrapper's `startup_progress_browser` (`-p figgy`) checks initialization event
compatibility. See [optimization measurements and limitations](shader-compilation.md).

## Run locally

Requires Rust 1.99.0 with the `rustfmt` component and the `wasm32-unknown-unknown`
target (all pinned in `rust-toolchain.toml`), and Python 3.12 or newer. On Linux, CI
also installs the Ubuntu packages `pkg-config libx11-dev libxkbcommon-dev
libwayland-dev libegl1-mesa-dev mesa-vulkan-drivers libvulkan1`. On a machine
without a render device, set `VK_DRIVER_FILES` (and `VK_ICD_FILENAMES` for older
Vulkan loaders) to Mesa's `lvp_icd*.json` to test on lavapipe. `ci/verify.py` does
not set `RUSTFLAGS` or `RUSTDOCFLAGS`; export both as `-D warnings` to fail on
warnings as CI does. `static` runs `cargo fmt --all --check` before any build.
From the checkout root:

```sh
python3 ci/verify.py static
python3 ci/verify.py packages
```

On Windows, use `python` if `python3` is unavailable. Select a supported native backend
with `WGPU_BACKEND=vulkan`, `dx12` or `metal`. Without that variable, wgpu chooses from
available backends. A working adapter and device are required; no window is needed.
Production adapter selection is not changed by this test configuration.

Each extraction gets its own build directory so cached binaries cannot retain paths
to a deleted extraction. The extracted workspace patches the renderer's `figgy-model`
dependency (`[patch.crates-io]`) to the extracted model package, so the tests never
use the crates.io copy; external dependency versions remain locked. `cargo package`
refuses uncommitted changes, so commit first or, to review local edits only, run
`python3 ci/verify.py packages --allow-dirty`. Results go to `target/ci-results/`.

Windows CI installs pinned DXC and WARP builds from Microsoft, verifies their SHA-256
checksums, and copies WARP beside the extracted test executables. Local Windows users
can use their installed driver instead; the setup script is specific to GitHub runners.

## 한국어 안내

CI는 공개 저장소에서 초안이 아닌 PR을 열거나 갱신하거나 다시 열 때, 그리고 초안 PR을
검토 준비 상태로 바꿀 때만 실행한다. 브랜치, master, 태그 push로는 시작하지 않는다.
로컬에서는 저장소 루트에서 `python3 ci/verify.py static`과 `python3 ci/verify.py packages`를
실행한다.

세 운영체제에서 네이티브 빌드와 WASM 컴파일(브라우저 테스트 대상 포함, 실행은 하지 않음), API 문서 링크 검사, 압축을 푼 패키지의
단위·통합·문서 테스트를 실행한다. Linux는 GPU가 없을 때 lavapipe를 쓰고,
Windows는 WARP, macOS는 러너에 노출된 Metal 장치를 쓴다. 유료 GPU 러너는 사용하지 않는다.
macOS / Metal은 아직 불안정한 실험 단계다. 정밀 Metal 셰이더 실험과 기존 전체 검사는
별도로 실행하며, 작은 실험의 성공으로 기존 실패를 통과 처리하지 않는다.

실제로 그린 픽셀, 설정 변경 후 화면, 캐시 재사용, 스트리밍과 메모리를 검사한다.
장치를 만들 수 없거나 렌더링에 실패하면 CI도 실패한다. 운영체제별 로그와 진단 이미지는
PR의 Actions 실행 아티팩트에서 확인할 수 있다. master 커밋은 따로 실행하지 않으므로 그 커밋을
병합한 PR의 마지막 실행 결과를 본다. 이 검사는 실제 GPU 제조사별 드라이버 검증이나
브라우저 실행 검사를 대신하지 않는다. [실행 결과와 확인된 문제](STATUS.md)도 함께 확인한다.
한 테스트가 실패해도 다른 테스트 바이너리와 문서 테스트는 계속 실행하며, 최종 CI 결과는
실패로 남는다.

셰이더 컴파일 시간도 실제 생성 경로에서 잰다. 모든 WGSL 진입점이 검사에 포함됐는지,
예열을 한 번 더 해도 셰이더를 새로 만들지 않는지 확인한다. 각 컴파일이 기본 제한인 30초를 넘으면
실패하며, 측정값은 `shader-compilation-native.json`으로 남긴다. 브라우저 계측과 실제
WASM·PNG 검사는 위 명령으로 별도 실행한다. [최적화 내용과 측정 범위](shader-compilation.md)를
참고한다.
