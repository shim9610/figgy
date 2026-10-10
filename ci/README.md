# Build and rendering verification

The public GitHub Actions workflow uses **standard hosted runners**:

| Runner | Requested backend | Rendering device |
| --- | --- | --- |
| `ubuntu-24.04` | Vulkan | Mesa lavapipe when no render device is present |
| `windows-2025` | DX12 | Microsoft WARP software adapter |
| `macos-15` (ARM64) | Metal | The runner's exposed Metal adapter, normally virtualized |

No larger, dedicated GPU or self-hosted runner is required. Standard runner execution
is free for public repositories under GitHub's current policy. The full matrix is
enabled only when the repository is public. These jobs cover
backend behavior; they do not certify physical Intel/AMD/NVIDIA/Apple GPU drivers.
The actual adapter and backend are printed in each run. A missing adapter, rejected
device or failed render **fails the job**; there is no successful CPU-only fallback
for the Metal job and no skipped GPU initialization.

macOS / Metal support is currently **experimental, not stable**. Its normal matrix
job continues to report failures. The separate, opt-in
[Metal precision experiment](metal-precision-probe/README.md) compares default WGSL
with fast and precise precompiled Metal shaders without patching wgpu. Success in
that narrowly scoped probe is not a passing renderer/package matrix.

## What runs on every OS

1. Native workspace checks with default and all optional features, including examples.
2. Compilation for `wasm32-unknown-unknown` and strict Rust API documentation links.
3. Creation and Cargo verification of both `.crate` archives, without registry upload.
4. Extraction outside the checkout, followed by model/renderer unit tests, integration
   tests and doctests with `serde` enabled. This includes native GPU pixel, SSOT edit,
   cache reuse, streaming, picking and memory regressions, plus radial, categorical
   and boxplot rendering tests.
5. Three subprocess checks with no enabled adapters: each must run its intended test
   and fail on adapter initialization. An unsupported device-limit request tests the
   device-error path. Linux additionally exercises an unavailable Vulkan driver.

Pixel checks compare rendered contents and edited/cached output with a fresh render
on the same backend. They are not an assertion that all drivers produce byte-identical
images. Existing opt-in stress/probe tests remain `#[ignore]` and are reported by Rust.
Tests run sequentially so GPU memory measurements do not overlap. These are headless
native renderer tests; WASM compilation is not a browser execution test.

Logs and generated diagnostic PNGs are uploaded as `verification-<runner>-<backend>`
artifacts, including on failure, and retained for seven days. The logs identify which
checks actually completed. The configured matrix alone is not evidence of a pass;
consult the commit's Actions run and the [recorded backend results](STATUS.md).
Independent test binaries, doctests and missing-adapter checks continue after a test
failure, while the final job still fails. This prevents one failing suite from hiding
the rest of the backend results.
Test output is streamed immediately, so a cancellation or timeout does not hide
panic details until the end of a large test binary.

## Shader compilation regressions

The renderer unit suite measures the real shader and pipeline constructors with
`Instant`. It covers every shipped WGSL module and entry point, Cartesian pipelines
at sample counts 1 and 4, streaming/replay, picking/fit, and the radial, categorical
and boxplot renderers. A missing source or entry fails coverage. Repeating
`prewarm_all` must create no shaders or pipelines; a single-pipeline builder must
load only its own shader. No timing instrumentation is included in native release
builds.

The package job saves `shader-compilation-native.json` in the existing CI artifact.
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

The test runs real window initialization and full prewarm, checks that a second
prewarm creates nothing, renders automatic contour labels, and decodes the exported
PNG to check its pixels. The existing wrapper test `startup_progress_browser` checks
initialization event compatibility. See [optimization measurements and limitations](shader-compilation.md).

## Run locally

Requires Rust 1.99, Python 3.12 or newer, and the `wasm32-unknown-unknown` target.
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
to a deleted extraction. The unpublished model dependency uses a local registry patch;
external dependency versions remain locked. `--allow-dirty` is only for reviewing local
edits. Results go to `target/ci-results/`. Repository-wide audits run separately from
these distributable package tests.

Windows CI installs pinned DXC and WARP builds from Microsoft, verifies their SHA-256
checksums, and copies WARP beside the extracted test executables. Local Windows users
can use their installed driver instead; the setup script is specific to GitHub runners.

## 한국어 안내

세 운영체제에서 네이티브 빌드, WASM 컴파일, API 문서 링크, 압축을 푼 패키지의
단위·통합·문서 테스트를 실행한다. Linux는 GPU가 없을 때 lavapipe를 쓰고,
Windows는 WARP, macOS는 러너에 노출된 Metal 장치를 쓴다. 유료 GPU 러너는 사용하지 않는다.
macOS / Metal은 아직 불안정한 실험 단계다. 정밀 Metal 셰이더 실험과 기존 전체 검사는
별도로 실행하며, 작은 실험의 성공으로 기존 실패를 통과 처리하지 않는다.

실제로 그린 픽셀, 설정 변경 후 화면, 캐시 재사용, 스트리밍과 메모리를 검사한다.
장치를 만들 수 없거나 렌더링에 실패하면 CI도 실패한다. 운영체제별 로그와 진단 이미지는
Actions 실행의 아티팩트에서 확인할 수 있다. 이 검사는 실제 GPU 제조사별 드라이버 검증이나
브라우저 실행 검사를 대신하지 않는다. [실행 결과와 확인된 문제](STATUS.md)도 함께 확인한다.
한 테스트가 실패해도 다른 테스트 바이너리와 문서 테스트는 계속 실행하며, 최종 CI 결과는
실패로 남는다.

셰이더 컴파일 시간도 실제 생성 경로에서 잰다. 모든 WGSL 진입점이 검사에 포함됐는지,
재예열 때 셰이더를 다시 만들지 않는지 확인한다. 각 컴파일이 기본 제한인 30초를 넘으면
실패하며, 측정값은 `shader-compilation-native.json`으로 남긴다. 브라우저 계측과 실제
WASM·PNG 검사는 위 명령으로 별도 실행한다. [최적화 내용과 측정 범위](shader-compilation.md)를
참고한다.
