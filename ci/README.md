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

실제로 그린 픽셀, 설정 변경 후 화면, 캐시 재사용, 스트리밍과 메모리를 검사한다.
장치를 만들 수 없거나 렌더링에 실패하면 CI도 실패한다. 운영체제별 로그와 진단 이미지는
Actions 실행의 아티팩트에서 확인할 수 있다. 이 검사는 실제 GPU 제조사별 드라이버 검증이나
브라우저 실행 검사를 대신하지 않는다. [실행 결과와 확인된 문제](STATUS.md)도 함께 확인한다.
한 테스트가 실패해도 다른 테스트 바이너리와 문서 테스트는 계속 실행하며, 최종 CI 결과는
실패로 남는다.
