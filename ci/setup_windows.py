#!/usr/bin/env python3
"""Install pinned Microsoft DXC/WARP into the runner's temporary directory."""
import hashlib
import io
import os
from pathlib import Path
import urllib.request
import zipfile


def extract_verified(url, sha256, members, destination):
    with urllib.request.urlopen(url, timeout=120) as response:
        archive = response.read()
    if hashlib.sha256(archive).hexdigest() != sha256:
        raise RuntimeError(f"checksum mismatch: {url}")
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(io.BytesIO(archive)) as package:
        for member in members:
            (destination / Path(member).name).write_bytes(package.read(member))


def main():
    destination = Path(os.environ["RUNNER_TEMP"]) / "figgy-dx12"
    extract_verified(
        "https://github.com/microsoft/DirectXShaderCompiler/releases/download/"
        "v1.9.2602.24/dxc_2026_05_27.zip",
        "cf658aacf070d3045e31b8f1f8a696c2945f37c1095019481ef7c513368db3b4",
        ["bin/x64/dxcompiler.dll", "bin/x64/dxil.dll"], destination,
    )
    extract_verified(
        "https://api.nuget.org/v3-flatcontainer/microsoft.direct3d.warp/"
        "1.0.20/microsoft.direct3d.warp.1.0.20.nupkg",
        "e5fe5de661ce98b58ef9cfb736e73c0a7a2623d3bbf5f14839b2d55566d87e40",
        ["build/native/bin/x64/d3d10warp.dll"], destination,
    )
    with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env:
        env.write(f"WGPU_DX12_COMPILER={destination / 'dxcompiler.dll'}\n")
        env.write(f"FIGGY_TEST_WARP_DLL={destination / 'd3d10warp.dll'}\n")
        env.write("FIGGY_TEST_FORCE_FALLBACK=1\n")
    print("Installed checksum-verified DXC and WARP; tests require the DX12 fallback adapter.")


if __name__ == "__main__":
    main()
