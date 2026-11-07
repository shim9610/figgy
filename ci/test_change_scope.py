"""Which pull requests may skip the package checks."""
import unittest

from change_scope import needs_checks


class ChangeScopeTests(unittest.TestCase):
    def test_standalone_markdown_skips(self):
        self.assertFalse(needs_checks(["README.md", "CHANGELOG.md", "ci/README.md",
                                       "tools/playground/README.md",
                                       ".github/PULL_REQUEST_TEMPLATE.md"]))

    def test_crate_markdown_runs(self):
        # Packaged and link-checked; the included guides are doctested.
        for path in ("crates/renderer/INTERACTION.md", "crates/model/README.md",
                     "crates/renderer/src/data_render/SHADER_COMMON.md"):
            self.assertTrue(needs_checks(["README.md", path]), path)

    def test_any_other_file_runs(self):
        for path in ("ci/verify.py", ".github/workflows/ci.yml", "Cargo.lock",
                     "crates/renderer/src/lib.rs", "LICENSE-MIT", "docs/chart.png"):
            self.assertTrue(needs_checks(["README.md", path]), path)

    def test_empty_change_runs(self):
        self.assertTrue(needs_checks([]))


if __name__ == "__main__":
    unittest.main()
