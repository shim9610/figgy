"""CI probe only: compile both shader sets' vector modules with the DXC that
the DX12 job uses, with wgpu's arguments, and compare the DXIL."""
import hashlib, io, os, re, subprocess, sys, urllib.request, zipfile
from pathlib import Path

here = Path(__file__).parent
out = Path(os.environ["RUNNER_TEMP"]) / "dxil"
out.mkdir(parents=True, exist_ok=True)
url = ("https://github.com/microsoft/DirectXShaderCompiler/releases/download/"
       "v1.9.2602.24/dxc_2026_05_27.zip")
data = urllib.request.urlopen(url, timeout=120).read()
assert hashlib.sha256(data).hexdigest() == "cf658aacf070d3045e31b8f1f8a696c2945f37c1095019481ef7c513368db3b4"
with zipfile.ZipFile(io.BytesIO(data)) as z:
    for m in ["bin/x64/dxc.exe", "bin/x64/dxcompiler.dll", "bin/x64/dxil.dll"]:
        (out / Path(m).name).write_bytes(z.read(m))
dxc = out / "dxc.exe"
ops = re.compile(r"@dx\.op\.(\w+)|\b(fadd|fsub|fmul|fdiv|fma|fcmp|select)\b")
for module in ["vector_points", "vector_points_3d"]:
    counts = {}
    for s in ["basic", "full"]:
        text = out / f"{module}-{s}.dxil.txt"
        subprocess.run([str(dxc), str(here / f"{module}-{s}.hlsl"), "-E", "vs_main", "-T", "vs_6_0",
                        "-HV", "2018", "-no-warnings", "-Ges", "-Fc", str(text)], check=True)
        body = text.read_text()
        c = {}
        for m in ops.finditer(body):
            k = m.group(1) or m.group(2)
            c[k] = c.get(k, 0) + 1
        counts[s] = c
        print(f"DXIL {module} {s}: lines {len(body.splitlines())}")
    keys = sorted(set(counts["basic"]) | set(counts["full"]))
    for k in keys:
        b, f = counts["basic"].get(k, 0), counts["full"].get(k, 0)
        print(f"DXIL {module} op {k}: basic {b} full {f}{'  <-- differs' if b != f else ''}")
