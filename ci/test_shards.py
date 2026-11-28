import unittest

import verify


class ShardTargets(unittest.TestCase):
    TARGETS = ["category_axis", "decoration_oracle", "pick_draw_agreement",
               "shader_set_scenes", "theme_matrix", "vector_export"]

    def test_every_binary_runs_in_exactly_one_shard(self):
        runs = {}
        for shard in ("main", *verify.SHARDS):
            args = verify.shard_targets(shard, self.TARGETS)
            names = [args[i + 1] for i, arg in enumerate(args) if arg == "--test"]
            for name in names:
                runs.setdefault(name, []).append(shard)
        self.assertEqual(sorted(runs), self.TARGETS)
        self.assertTrue(all(len(shards) == 1 for shards in runs.values()), runs)

    def test_the_library_runs_only_in_its_shards_and_all_runs_everything(self):
        for shard in verify.LIB_SHARDS:
            self.assertEqual(verify.shard_targets(shard, self.TARGETS), ["--lib"])
        for shard in ("main", *verify.SHARDS):
            self.assertNotIn("--lib", verify.shard_targets(shard, self.TARGETS))
        self.assertEqual(verify.shard_targets("all", self.TARGETS), ["--lib", "--tests"])

    def test_the_library_shards_split_the_tests_exactly(self):
        lib = verify.shard_filters("lib")
        rest = verify.shard_filters("lib-rest")
        self.assertEqual(lib, list(verify.LIB_MODULES))
        self.assertEqual(rest[0::2], ["--skip"] * len(verify.LIB_MODULES))
        self.assertEqual(rest[1::2], lib)
        for shard in ("all", "main", *verify.SHARDS):
            self.assertEqual(verify.shard_filters(shard), [])

    def test_a_renamed_shard_binary_cannot_drop_out(self):
        with self.assertRaises(RuntimeError):
            verify.shard_targets("main", ["category_axis", "vector_export"])


if __name__ == "__main__":
    unittest.main()
