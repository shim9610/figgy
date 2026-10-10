#!/usr/bin/env python3
"""Validate sources or extracted registry packages; never upload packages."""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = ("figgy-model", "figgy-renderer")
RESULTS = ROOT / "target" / "ci-results"
LOG = None


def report(message):
    print(message, flush=True)
    if LOG is not None:
        LOG.write(message + "\n")
        LOG.flush()


def run(args, *, cwd=ROOT, env=None, capture=False):
    report(f"+ {shlex.join(map(str, args))}")
    if capture:
        return subprocess.run(args, cwd=cwd, env=env, check=True, text=True,
                              encoding="utf-8", stdout=subprocess.PIPE).stdout
    with subprocess.Popen(args, cwd=cwd, env=env, text=True, encoding="utf-8",
                          errors="replace", stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT) as process:
        for line in process.stdout:
            report(line.rstrip("\r\n"))
        if process.wait():
            raise subprocess.CalledProcessError(process.returncode, args)


def static():
    run(["cargo", "check", "--locked", "--workspace", "--all-targets"])
    run(["cargo", "check", "--locked", "--workspace", "--all-targets", "--all-features"])
    run(["cargo", "check", "--locked", "--workspace", "--target", "wasm32-unknown-unknown"])
    env = os.environ.copy()
    env["RUSTDOCFLAGS"] = env.get("RUSTDOCFLAGS", "") + " -D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links"
    run(["cargo", "doc", "--locked", "--no-deps", "--all-features",
         "-p", PACKAGES[0], "-p", PACKAGES[1]], env=env)


def locked_external_packages(lock):
    return {(p["name"], p["version"], p["source"], p.get("checksum"))
            for p in tomllib.loads(lock.read_text())["package"] if "source" in p}


def gpu_failure_checks(cwd, *, missing_driver=None):
    env = os.environ.copy()
    if missing_driver is None:
        env["FIGGY_TEST_DISABLE_ADAPTERS"] = "1"
    else:
        env.update(WGPU_BACKEND="vulkan", VK_DRIVER_FILES=str(missing_driver),
                   VK_ICD_FILENAMES=str(missing_driver))
    cases = [
        (["--test", "gpu_required"], "required_gpu_initialization"),
        (["--test", "memory_scaling"],
         "gpu_occupancy_is_linear_in_input_bytes_at_two_bytes_per_f32_byte"),
        (["--lib"], "data_render::tests::rgba_texture_upload_roundtrips_api"),
    ]
    for target, name in cases:
        args = ["cargo", "test", "--offline", "--locked", "-p", "figgy-renderer",
                "--features", "serde", *target, name, "--", "--exact", "--nocapture"]
        report(f"+ expected GPU failure: {shlex.join(args)}")
        result = subprocess.run(args, cwd=cwd, env=env, text=True,
                                encoding="utf-8", errors="replace",
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        output = result.stdout
        if (result.returncode == 0 or "running 1 test" not in output
                or "required GPU test: adapter initialization failed" not in output
                or f"test {name} ... FAILED" not in output):
            report(output)
            raise RuntimeError(f"{name}: missing driver did not fail the intended GPU test")
        report(f"PASS: {name} failed on adapter initialization (exit {result.returncode})")


def packages(allow_dirty):
    # Fetch the locked graph, including test dependencies, before offline extraction.
    run(["cargo", "fetch", "--locked"])
    metadata = json.loads(run(["cargo", "metadata", "--locked", "--no-deps",
                               "--format-version", "1"], capture=True))
    by_name = {p["name"]: p for p in metadata["packages"]}
    args = ["cargo", "package", "--locked", "-p", PACKAGES[0], "-p", PACKAGES[1]]
    if allow_dirty:
        args.append("--allow-dirty")
    run(args)
    # A directory outside the checkout catches accidental sibling/source dependencies.
    with tempfile.TemporaryDirectory(prefix="figgy-packages-") as directory:
        extracted = Path(directory).resolve()
        if extracted.is_relative_to(ROOT):
            raise RuntimeError("TMPDIR must be outside the source checkout")
        # Test binaries embed CARGO_MANIFEST_DIR. Reusing a target directory from
        # another temporary checkout can retain its now-deleted absolute paths.
        # Give this extraction its own build output as well as its own sources.
        os.environ["CARGO_TARGET_DIR"] = str(extracted / "target")
        if os.name == "nt" and (warp := os.environ.get("FIGGY_TEST_WARP_DLL")):
            for path in (extracted / "target/debug", extracted / "target/debug/deps"):
                path.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(warp, path / "d3d10warp.dll")
        members = [f"{name}-{by_name[name]['version']}" for name in PACKAGES]
        for member in members:
            archive = Path(metadata["target_directory"]) / "package" / f"{member}.crate"
            with tarfile.open(archive) as package:
                package.extractall(extracted, filter="data")
        (extracted / "Cargo.toml").write_text(
            '[workspace]\nresolver = "3"\nmembers = ' + json.dumps(members)
            + '\n[patch.crates-io]\nfiggy-model = { path = ' + json.dumps(members[0]) + ' }\n')
        shutil.copyfile(ROOT / "Cargo.lock", extracted / "Cargo.lock")
        # Normalize only local workspace identities; no external dependency upgrades.
        run(["cargo", "metadata", "--offline", "--format-version", "1"],
            cwd=extracted, capture=True)
        added = locked_external_packages(extracted / "Cargo.lock") - locked_external_packages(ROOT / "Cargo.lock")
        if added:
            raise RuntimeError(f"extracted tests changed locked dependencies: {added}")
        features = "figgy-model/serde,figgy-renderer/serde"
        base = ["cargo", "test", "--offline", "--locked", "--workspace", "--features", features]
        # Record the actual backend and adapter even when all tests pass.
        run(["cargo", "test", "--offline", "--locked", "-p", "figgy-renderer",
             "--features", "serde", "--test", "gpu_required", "--", "--nocapture",
             "--test-threads=1"], cwd=extracted)
        # A failed unit-test binary must not hide independent chart integration
        # tests or doctests. Collect failures, but keep the final exit nonzero.
        failures = []
        for args in (
            [*base, "--no-fail-fast", "--lib", "--tests", "--", "--test-threads=1", "--nocapture"],
            [*base, "--no-fail-fast", "--doc"],
        ):
            try:
                run(args, cwd=extracted)
            except subprocess.CalledProcessError as error:
                failures.append(str(error))
        try:
            gpu_failure_checks(extracted)
            if sys.platform.startswith("linux"):
                gpu_failure_checks(extracted, missing_driver=extracted / "missing-vulkan-driver.json")
        except RuntimeError as error:
            failures.append(str(error))
        if failures:
            raise RuntimeError("Package verification failed:\n" + "\n".join(failures))
        report("PASS: extracted package tests, doctests, and missing-GPU failure checks")


def main():
    global LOG
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("static", "packages"))
    parser.add_argument("--allow-dirty", action="store_true",
                        help="allow local uncommitted sources when creating archives")
    args = parser.parse_args()
    RESULTS.mkdir(parents=True, exist_ok=True)
    images = RESULTS / "images"
    images.mkdir(exist_ok=True)
    for key in ("FIGGY_PROBE_DIR", "FIGGY_LAYOUT_PROBE_DIR", "FIGGY_FIELD_DIAGNOSTIC_ARTIFACT_DIR"):
        os.environ.setdefault(key, str(images))
    with (RESULTS / f"{args.stage}.log").open("w", encoding="utf-8") as log:
        LOG = log
        report(f"Platform: {sys.platform}; requested backend: {os.environ.get('WGPU_BACKEND', 'automatic')}")
        try:
            if args.stage == "static":
                static()
            else:
                packages(args.allow_dirty)
        except Exception as error:
            report(f"FAIL: {error}")
            raise
        finally:
            LOG = None


if __name__ == "__main__":
    main()
