#!/usr/bin/env python3
"""Tell CI whether a pull request changes more than standalone Markdown."""
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


def changed_paths():
    # The pull_request checkout is the merge commit; its first parent is the
    # base branch. --no-renames lists both sides of a move.
    output = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "HEAD^1", "HEAD"],
        check=True, text=True, encoding="utf-8", stdout=subprocess.PIPE).stdout
    return [line for line in output.splitlines() if line]


def main():
    paths = changed_paths()
    checks = needs_checks(paths)
    for path in paths:
        print(path)
    print("Run the package checks." if checks
          else "Only standalone Markdown changed; skipping the package checks.")
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"checks={'true' if checks else 'false'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
