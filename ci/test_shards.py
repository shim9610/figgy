import unittest

import verify


class ShardTargets(unittest.TestCase):
    TARGETS = ["category_axis", "decoration_oracle", "pick_draw_agreement", "vector_export"]

    def test_every_binary_runs_in_exactly_one_shard(self):
        runs = {}
        for shard in ("main", *verify.SHARDS):
            args = verify.shard_targets(shard, self.TARGETS)
            names = [args[i + 1] for i, arg in enumerate(args) if arg == "--test"]
            for name in names:
                runs.setdefault(name, []).append(shard)
        self.assertEqual(sorted(runs), self.TARGETS)
        self.assertTrue(all(len(shards) == 1 for shards in runs.values()), runs)

    def test_main_runs_the_library_and_all_runs_everything(self):
        self.assertEqual(verify.shard_targets("main", self.TARGETS)[0], "--lib")
        self.assertEqual(verify.shard_targets("all", self.TARGETS), ["--lib", "--tests"])
        self.assertNotIn("--lib", verify.shard_targets("picks", self.TARGETS))

    def test_a_renamed_shard_binary_cannot_drop_out(self):
        with self.assertRaises(RuntimeError):
            verify.shard_targets("main", ["category_axis", "vector_export"])


if __name__ == "__main__":
    unittest.main()
