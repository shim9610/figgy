#!/usr/bin/env python3
"""Tell CI whether a pull request needs the package checks or the browser tests.

`python ci/change_scope.py` asks whether the change is more than standalone
Markdown; `--browser` asks whether it can affect the WASM browser tests.
"""
import os
from pathlib import PurePosixPath
import subprocess
import sys

# Markdown under crates/ is packaged, linted by package_docs.py and, for the
# guides the crate docs include through include_str!, compiled as doctests.
CHECKED_DIRS = ("crates",)


def needs_checks(paths):
    """True unless every path is Markdown that no build, test or package reads."""
    if not paths:
        return True
    for path in paths:
        name = PurePosixPath(path)
        if name.suffix.lower() != ".md" or name.parts[0] in CHECKED_DIRS:
            return True
    return False


# What the `*_browser` test binaries and their runner are built from: the
# workspace crates, the build configuration, the runner script and the job.
BROWSER_DIRS = ("crates",)
BROWSER_FILES = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "clippy.toml",
                 "ci/wasm_browser_test.cjs", "ci/change_scope.py",
                 ".github/workflows/ci.yml")


def needs_browser_tests(paths):
    """True if any path can change what the WASM browser tests build or run."""
    if not paths:
        return True
    for path in paths:
        name = PurePosixPath(path)
        if name.parts[0] in BROWSER_DIRS or path in BROWSER_FILES:
            return True
    return False


def changed_paths():
    # The pull_request checkout is the merge commit; its first parent is the
    # base branch. --no-renames lists both sides of a move.
    output = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "HEAD^1", "HEAD"],
        check=True, text=True, encoding="utf-8", stdout=subprocess.PIPE).stdout
    return [line for line in output.splitlines() if line]


def main(argv):
    paths = changed_paths()
    for path in paths:
        print(path)
    if argv == ["--browser"]:
        checks = needs_browser_tests(paths)
        print("Run the browser tests." if checks
              else "Nothing the browser tests build changed; skipping them.")
    elif not argv:
        checks = needs_checks(paths)
        print("Run the package checks." if checks
              else "Only standalone Markdown changed; skipping the package checks.")
    else:
        sys.exit("Usage: change_scope.py [--browser]")
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"checks={'true' if checks else 'false'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
