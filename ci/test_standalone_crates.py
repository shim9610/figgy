import tempfile
import unittest
from pathlib import Path

import verify


class StandaloneCrates(unittest.TestCase):
    def test_the_repository_standalone_crates_are_found(self):
        names = sorted(m.parent.relative_to(verify.ROOT).as_posix()
                       for m in verify.standalone_crates())
        self.assertEqual(names, ["examples/egui", "tools/loom-poller", "tools/playground"])

    def test_only_crates_with_their_own_workspace_and_lock(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name, manifest, lock in (("own", "[package]\n\n[workspace]\n", True),
                                         ("member", "[package]\n", True),
                                         ("unlocked", "[package]\n\n[workspace]\n", False)):
                crate = root / "tools" / name
                crate.mkdir(parents=True)
                (crate / "Cargo.toml").write_text(manifest)
                if lock:
                    (crate / "Cargo.lock").write_text("")
            found = [m.parent.name for m in verify.standalone_crates(root)]
            self.assertEqual(found, ["own"])


if __name__ == "__main__":
    unittest.main()
