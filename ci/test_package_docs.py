"""Package-boundary regressions, including Git's real pre-push entry point."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from package_docs import ROOT, audit_document_links, check_checkout


class PackageDocumentationTests(unittest.TestCase):
    def test_real_packages_with_empty_cargo_cache(self):
        # Hosted runners start without a registry index. Test the actual
        # manifests, including serde and the unpublished model dependency.
        with tempfile.TemporaryDirectory(prefix="figgy-empty-cargo-") as cache:
            with patch.dict(os.environ, {"CARGO_HOME": cache}):
                check_checkout(ROOT)
            self.assertFalse((Path(cache) / "registry" / "index").exists())

    def test_repository_sibling_is_not_a_package_link(self):
        documents = {"WASM.md": "[selection](../web/SCHEMA.md#resident-selection-details)"}
        with self.assertRaisesRegex(RuntimeError, "WASM.md links outside package: ../web/SCHEMA.md"):
            audit_document_links("renderer", documents, documents.__getitem__)

    def test_public_in_package_encoded_and_html_links(self):
        documents = {
            "README.md": '[API](API.md#usage) [source](https://github.com/shim9610/figgy) '
                         '[directory](examples/) [anchor](#here) '
                         '<img src="images/some%20plot.svg">',
            "API.md": "[home](README.md)",
            "examples/line.rs": "", "images/some plot.svg": "",
        }
        self.assertEqual(audit_document_links("renderer", documents, documents.__getitem__), 4)

    def test_hook_checks_outgoing_branch_not_repaired_worktree(self):
        with tempfile.TemporaryDirectory(prefix="figgy-hook-test-") as temporary:
            root = Path(temporary)
            repo, remote = root / "repo", root / "remote.git"
            repo.mkdir()
            env = dict(os.environ, CARGO_HOME=str(root / "empty-cargo"))

            def git(*args, check=True):
                return subprocess.run(["git", *args], cwd=repo, check=check, text=True,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)

            git("init", "-q")
            git("config", "user.name", "Hook regression")
            git("config", "user.email", "hook-test@example.invalid")
            git("config", "core.hooksPath", ".githooks")
            git("init", "--bare", "-q", str(remote))
            git("remote", "add", "origin", str(remote))
            (repo / "Cargo.toml").write_text(
                '[workspace]\nresolver = "3"\nmembers = ["crates/model", "crates/renderer"]\n')
            for name in ("model", "renderer"):
                package = repo / "crates" / name
                (package / "src").mkdir(parents=True)
                (package / "src/lib.rs").write_text("")
                (package / "Cargo.toml").write_text(
                    f'[package]\nname = "figgy-{name}"\nversion = "0.0.0"\n'
                    'edition = "2024"\ninclude = ["/*.md", "/src/**"]\n'
                    '[dependencies]\nserde = "1"\n')
                (package / "README.md").write_text("[source](https://example.org)\n")
            # This file exists in the repository, but not the renderer package.
            (repo / "crates/web").mkdir()
            (repo / "crates/web/SCHEMA.md").write_text("Selection schema\n")
            document = repo / "crates/renderer/WASM.md"
            document.write_text("[selection](../web/SCHEMA.md)\n")
            for relative in (".githooks/pre-push", "ci/package_docs.py"):
                destination = repo / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / relative, destination)
            git("add", ".")
            git("commit", "-qm", "Broken package link")
            git("branch", "broken-docs")
            document.write_text("[selection](https://example.org/SCHEMA.md)\n")
            git("add", ".")
            git("commit", "-qm", "Repair working branch")
            rejected = git("push", "origin", "broken-docs", check=False)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("WASM.md links outside package", rejected.stderr)
            self.assertEqual(git("ls-remote", "origin", "refs/heads/broken-docs").stdout, "")
            accepted = git("push", "origin", "HEAD:refs/heads/good-docs", check=False)
            self.assertEqual(accepted.returncode, 0, accepted.stderr)
            # A deletion has no outgoing source tree and must remain possible.
            deleted = git("push", "origin", ":refs/heads/good-docs", check=False)
            self.assertEqual(deleted.returncode, 0, deleted.stderr)


if __name__ == "__main__":
    unittest.main()
