"""DIAGNOSTIC ONLY (diag/backend-pick, never merged).

Runs the tests that fail on DX12/WARP and Metal but pass on lavapipe, with
image and text dumps under target/ci-results/diag, so the backends can be
compared.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "ci-results" / "diag"
OUT.mkdir(parents=True, exist_ok=True)
env = dict(os.environ, FIGGY_DIAG_DIR=str(OUT), RUST_BACKTRACE="1")
base = ["cargo", "test", "--locked", "-p", "figgy-renderer",
        "--features", "serde,cpu-twin", "--no-fail-fast"]
runs = [
    ["--test", "radial_render"],
    ["--test", "radial_render", "--", "radial_gpu_pick_agrees_with_the_cpu_twin"],
    ["--test", "radial_render", "--", "radial_step_selection_moves_the_highlight_to_each_slice"],
]
# Build first so the WARP and DXC DLLs can sit next to the test binaries.
subprocess.run([*base, "--lib", "--tests", "--no-run"], cwd=ROOT, env=env, check=True)
if os.name == "nt" and (warp := os.environ.get("FIGGY_TEST_WARP_DLL")):
    for path in (ROOT / "target/debug", ROOT / "target/debug/deps"):
        shutil.copyfile(warp, path / "d3d10warp.dll")
        if dxc := os.environ.get("FIGGY_TEST_DXC_DIR"):
            for dll in ("dxcompiler.dll", "dxil.dll"):
                shutil.copyfile(Path(dxc) / dll, path / dll)
failed = False
with open(OUT / "log.txt", "w", encoding="utf-8") as log:
    for args in runs:
        cmd = [*base, *args, "--nocapture", "--test-threads=1"]
        log.write("+ " + " ".join(cmd) + "\n")
        log.flush()
        result = subprocess.run(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                                errors="replace")
        log.write(result.stdout)
        sys.stdout.write(result.stdout)
        failed |= result.returncode != 0
sys.exit(1 if failed else 0)
