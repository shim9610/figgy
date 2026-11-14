"""The GPU numeric step: every listed test must run and pass, red raises at once."""
import subprocess
import unittest
from unittest import mock

import verify


def _ok(target, name):
    if name is None:
        return "running 3 tests\ntest a ... ok\ntest result: ok. 3 passed\n"
    # `--nocapture` output lands between the name and the verdict.
    return f"running 1 test\ntest {name} ... adapter: llvmpipe\nok\ntest result: ok. 1 passed\n"


class NumericChecksTests(unittest.TestCase):
    def test_every_listed_test_runs_and_passes(self):
        calls = []

        def fake_run(args, *, cwd=None, collect=False, **_):
            calls.append(args)
            features, target, name = verify.NUMERIC_TESTS[len(calls) - 1]
            self.assertEqual(args[args.index("--features") + 1], features)
            self.assertEqual(args[args.index("--features") + 2:][:len(target)], target)
            return _ok(target, name)

        with mock.patch.object(verify, "run", fake_run), mock.patch.object(verify, "report"):
            verify.numeric_checks("cwd")
        self.assertEqual(len(calls), len(verify.NUMERIC_TESTS))
        for args in calls:
            self.assertIn("--locked", args)
            self.assertIn("--test-threads=1", args)
        self.assertIn("serde,cpu-twin", calls[0])

    def test_a_red_test_raises_at_once(self):
        calls = []

        def fake_run(args, **_):
            calls.append(args)
            raise subprocess.CalledProcessError(101, args)

        with mock.patch.object(verify, "run", fake_run), mock.patch.object(verify, "report"):
            with self.assertRaisesRegex(RuntimeError, "GPU numeric test red"):
                verify.numeric_checks("cwd")
        self.assertEqual(len(calls), 1)

    def test_a_renamed_test_cannot_drop_out(self):
        def fake_run(args, **_):
            return "running 0 tests\ntest result: ok. 0 passed\n"

        with mock.patch.object(verify, "run", fake_run), mock.patch.object(verify, "report"):
            with self.assertRaisesRegex(RuntimeError, "did not run"):
                verify.numeric_checks("cwd")


if __name__ == "__main__":
    unittest.main()
