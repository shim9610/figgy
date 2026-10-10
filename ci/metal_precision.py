"""Opt-in precision experiment. Never patches wgpu or changes renderer defaults."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/ci-results/metal-precision"
MANIFEST = ROOT / "ci/metal-precision-probe/Cargo.toml"


def run(args):
    print("+", " ".join(map(str, args)), flush=True)
    subprocess.run(args, cwd=ROOT, check=True)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    cargo = ["cargo", "run", "--locked", "--manifest-path", str(MANIFEST), "--"]
    run([*cargo, "generate", str(OUT), str(ROOT)])
    if sys.argv[1:] == ["--generate-only"]:
        return
    if sys.argv[1:] == ["--vulkan-control"]:
        run([*cargo, "vulkan", str(OUT)])
        return
    if sys.argv[1:]:
        raise SystemExit("usage: metal_precision.py [--generate-only | --vulkan-control]")
    if sys.platform != "darwin":
        raise SystemExit("Metal execution requires macOS; generation is not a passing Metal test")
    run(["xcrun", "--sdk", "macosx", "metal", "--version"])
    commands = []
    for variant, math_flag in [("fast", "-ffast-math"), ("precise", "-fno-fast-math")]:
        compile_cmd = ["xcrun", "--sdk", "macosx", "metal", "-std=macos-metal2.3",
                       "-mmacosx-version-min=11.0", math_flag, "-c", str(OUT / "probe.metal"),
                       "-o", str(OUT / f"{variant}.air")]
        run(compile_cmd)
        run(["xcrun", "--sdk", "macosx", "metallib", str(OUT / f"{variant}.air"),
             "-o", str(OUT / f"{variant}.metallib")])
        commands.append(compile_cmd)
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in OUT.iterdir()
              if p.suffix in {".wgsl", ".metal", ".metallib"}}
    (OUT / "build.json").write_text(json.dumps({"commands": commands, "sha256": hashes}, indent=2))
    run([*cargo, "metal", str(OUT)])


if __name__ == "__main__":
    main()
