"""Which pull requests may skip the package checks."""
import unittest

from change_scope import needs_browser_tests, needs_checks


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


class BrowserScopeTests(unittest.TestCase):
    def test_other_files_skip(self):
        self.assertFalse(needs_browser_tests([
            "README.md", "ci/verify.py", "ci/shader_compile.cjs", "tools/playground/src/main.rs",
            "examples/egui/src/main.rs", "docs/chart.png", ".github/PULL_REQUEST_TEMPLATE.md"]))

    def test_build_inputs_run(self):
        for path in ("crates/web/src/lib.rs", "crates/renderer/INTERACTION.md",
                     "crates/web/tests/hit_interaction_browser.rs", "Cargo.toml", "Cargo.lock",
                     "rust-toolchain.toml", "clippy.toml", "ci/wasm_browser_test.cjs",
                     "ci/change_scope.py", ".github/workflows/ci.yml"):
            self.assertTrue(needs_browser_tests(["README.md", path]), path)

    def test_empty_change_runs(self):
        self.assertTrue(needs_browser_tests([]))


if __name__ == "__main__":
    unittest.main()
