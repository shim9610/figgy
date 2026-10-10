"""Temporary backend investigation; not a release or a replacement CI gate."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/ci-results"
OUT.mkdir(parents=True, exist_ok=True)
RESULTS = []
PICK = ["gpu_pick::tests::exact_gpu_f64_residual_selects_point_index",
        "gpu_pick::tests::exact_gpu_line_boundary_endpoint_matches_cpu_exhaustive_picker"]
PIXEL = "renderer::streaming_runtime::selection::tests::stream_selection_and_data_match_resident_at_exact_display_scale"
STYLE = "data_render::stream_point_style_tests::tests::streamed_style_point_and_errorbar_entries_match_resident_pixels"


def command(args, name, extra=None):
    if os.name == "nt":
        name = name.replace("baseline-", "explicit-dxc-")
    env = os.environ.copy()
    env.update(extra or {})
    env["FIGGY_DIAG_IMAGES"] = str(OUT / name)
    start = time.monotonic()
    print("DIAGNOSTIC", name, args, extra, flush=True)
    with (OUT / f"{name}.log").open("w", encoding="utf-8") as log:
        with subprocess.Popen(args, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                              errors="replace") as process:
            for line in process.stdout:
                print(line.rstrip(), flush=True)
                log.write(line)
                log.flush()
            code = process.wait()
    RESULTS.append(dict(name=name, command=args, environment=extra or {}, exit_code=code,
                        seconds=round(time.monotonic() - start, 2)))
    (OUT / "diagnostics.json").write_text(json.dumps(RESULTS, indent=2))
    return code


def test(name, filter_name, extra=None, target=None):
    args = ["cargo", "test", "--locked", "-p", "figgy-renderer", "--features", "serde"]
    args += ["--test", target] if target else ["--lib"]
    return command([*args, filter_name, "--", "--exact", "--nocapture", "--test-threads=1"], name, extra)


if os.name == "nt":
    for path in (ROOT / "target/debug", ROOT / "target/debug/deps"):
        path.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(os.environ["FIGGY_TEST_WARP_DLL"], path / "d3d10warp.dll")
        for dll in ("dxcompiler.dll", "dxil.dll"):
            shutil.copyfile(Path(os.environ["FIGGY_DIAG_DXC_DIR"]) / dll, path / dll)
    print("Controlled DXC selection:", os.environ["WGPU_DX12_COMPILER"], "DLLs next to test executables", flush=True)

for i, name in enumerate(PICK):
    test(f"baseline-pick-{i}", name)
test("baseline-pixels", PIXEL)
test("baseline-styled-stream", STYLE)
test("baseline-stream-start", "renderer::streaming_request_tests::automatic_range_request_is_stable_until_submit_advances_the_cpu_cursor")
test("baseline-prewarm", "renderer::tests::full_prewarm_materializes_every_deferred_pipeline")
test("baseline-field-locate", "bounded_gpu_axis_replay_matches_resident_global_locate", target="stream_field_locate_replay")

if os.name == "nt":
    test("explicit-dxc-pixel-no-grid", PIXEL, {"FIGGY_DIAG_DATA_ONLY": "1", "FIGGY_DIAG_NO_GRID": "1"})
    test("explicit-dxc-pixel-opaque", PIXEL, {"FIGGY_DIAG_DATA_ONLY": "1", "FIGGY_DIAG_OPAQUE": "1"})

if sys.platform == "darwin":
    for variant, extra in [
        ("opaque", {"FIGGY_DIAG_OPAQUE": "1"}),
        ("scatter-only", {"FIGGY_DIAG_SERIES": "scatter"}),
        ("bars-only", {"FIGGY_DIAG_SERIES": "bars"}),
        ("no-grid", {"FIGGY_DIAG_NO_GRID": "1"}),
    ]:
        test(f"pixel-{variant}", PIXEL, {"FIGGY_DIAG_DATA_ONLY": "1", **extra})
    for i, name in enumerate([
        "renderer::streaming_runtime::selection::tests::stream_selection_preserves_data_ticket_and_atomically_replaces_ordered_suffix",
        "renderer::streaming_runtime::selection::tests::stream_selection_suspension_releases_single_slot_for_export_and_resumes_candidate",
        "renderer::streaming_surface::replay::tests::progressive_replay_selects_one_image_without_double_alpha_or_blank_suffix",
        "text_render::tests::register_font_bytes_resolves_new_family_and_publishes_generation",
    ]):
        test(f"isolated-cascade-{i}", name)

    # Controlled experiment only: change the actual Metal compiler option in a
    # temporary copy of the locked wgpu-hal dependency. No application WGSL or
    # floating-point comparison tolerance is changed for this A/B test.
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], text=True))
    hal = next(p for p in metadata["packages"] if p["name"] == "wgpu-hal" and p["version"] == "30.0.1")
    patch = Path(os.environ["RUNNER_TEMP"]) / "figgy-diagnostic-wgpu-hal"
    shutil.copytree(Path(hal["manifest_path"]).parent, patch)
    device = patch / "src/metal/device.rs"
    source = device.read_text()
    anchor = "let options = MTLCompileOptions::new();"
    assert source.count(anchor) == 2
    device.write_text(source.replace(anchor, anchor + "\n                options.setFastMathEnabled(false);"))
    with (ROOT / "Cargo.toml").open("a") as manifest:
        manifest.write('\n[patch.crates-io]\nwgpu-hal = { path = ' + json.dumps(str(patch)) + ' }\n')
    subprocess.run(["cargo", "metadata", "--offline", "--format-version", "1"], stdout=subprocess.DEVNULL, check=True)
    for i, name in enumerate(PICK):
        test(f"metal-no-fast-math-pick-{i}", name)
    test("metal-no-fast-math-pixels", PIXEL)
    test("metal-no-fast-math-field-locate", "bounded_gpu_axis_replay_matches_resident_global_locate", target="stream_field_locate_replay")

print(json.dumps(RESULTS, indent=2), flush=True)
# Diagnostic variants never turn known baseline failures into a green gate.
sys.exit(1 if any(r["exit_code"] for r in RESULTS) else 0)
