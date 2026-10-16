#!/usr/bin/env python3
"""Check distributed Markdown links without building or accessing the network."""
import argparse
from pathlib import Path
import posixpath
import re
import subprocess
import sys
import tarfile
import tempfile
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
PACKAGE_DIRS = ("crates/model", "crates/renderer")


def audit_document_links(package, files, read_text):
    """Shared source/archive check: relative links must stay in the package."""
    links = 0
    for name in sorted(files):
        if not name.endswith(".md"):
            continue
        targets = re.findall(r'\]\(([^\s)]+)\)|(?:src|href)="([^"]+)"', read_text(name))
        for markdown, html in targets:
            url = urlsplit(markdown or html)
            if url.scheme or url.netloc or not url.path:
                continue
            target = posixpath.normpath(posixpath.join(
                posixpath.dirname(name), unquote(url.path)))
            if target not in files and not any(n.startswith(target + "/") for n in files):
                raise RuntimeError(f"{package}: {name} links outside package: {url.path}")
            links += 1
    return links


def check_checkout(root):
    for directory in PACKAGE_DIRS:
        source = root / directory
        # --list alone still resolves dependencies to generate Cargo.lock.
        # Exclude that generated file so even an empty Cargo cache works
        # offline; all source files still follow Cargo's include/exclude rules.
        result = subprocess.run(
            ["cargo", "package", "--list", "--offline", "--allow-dirty",
             "--exclude-lockfile", "--manifest-path", str(source / "Cargo.toml")],
            cwd=root, check=True, text=True, encoding="utf-8", stdout=subprocess.PIPE)
        # Cargo prints native paths on Windows; Markdown and tar entries use
        # forward slashes on every platform.
        files = {name.replace("\\", "/") for name in result.stdout.splitlines()}
        # Normal release packaging includes this generated file, so links to
        # it are valid. Actual archive validation checks that it is present.
        files.add("Cargo.lock")
        links = audit_document_links(directory, files,
                                     lambda name: (source / name).read_text(encoding="utf-8"))
        print(f"PASS: {directory}: {links} package-local documentation links", flush=True)


def check_push(root, updates):
    """Audit each outgoing tip, even when it is not the checked-out branch."""
    checked = set()
    for line in updates:
        fields = line.split()
        if len(fields) != 4:
            raise RuntimeError("Malformed Git pre-push update")
        local_ref, oid, _remote_ref, _remote_oid = fields
        if not oid.strip("0") or oid in checked:
            continue  # deletion or an already checked tip
        checked.add(oid)
        print(f"Checking outgoing {local_ref} ({oid[:12]})", flush=True)
        with tempfile.TemporaryDirectory(prefix="figgy-push-docs-") as temporary:
            directory = Path(temporary)
            archive = directory / "source.tar"
            subprocess.run(["git", "archive", "--format=tar", f"--output={archive}", oid],
                           cwd=root, check=True)
            checkout = directory / "checkout"
            checkout.mkdir()
            with tarfile.open(archive) as source:
                source.extractall(checkout, filter="data")
            check_checkout(checkout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pre-push", action="store_true", help="read outgoing Git refs from stdin")
    args = parser.parse_args()
    try:
        if args.pre_push:
            check_push(ROOT, sys.stdin)
        else:
            check_checkout(ROOT)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Package documentation check failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
