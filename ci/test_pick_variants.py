"""The exact pick variant table (#363) belongs to the current shaders.

`crates/renderer/tests/support/pick_variants.csv` lists every exact pick
pipeline a chart can reach with its measured SwiftShader compile time. For
each exact module and shader set it records the hash of the assembled
source its rows were measured on and the fragment files that source is
assembled from (joined with one newline). This test rehashes those
fragments: a change to an exact module fails until that module's rows are
regenerated and re-measured, and a WGSL change outside the exact modules
changes nothing. The rows themselves, and the fragment lists, are checked
against the renderer by its `pick_variant_table_matches_the_picker` test.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
CRATE = ROOT / "crates" / "renderer"
TABLE = CRATE / "tests" / "support" / "pick_variants.csv"
COLUMNS = ("module,set,bits,bits_names,shape,constants,families,example,cases,"
           "measured_ms,budget_ms")
REGENERATE = ("FIGGY_UPDATE_PICK_VARIANTS=1 cargo test -p figgy-renderer --lib "
              "pick_variant_table_matches_the_picker")
MEASURE = ("node ci/shader_compile.cjs --pick-variants "
           "target/ci-results/shader-compilation-native.json "
           "crates/renderer/tests/support/pick_variants.csv --update")


def fnv1a(hash_value, data):
    """64-bit FNV-1a, as the Rust table test and ci/shader_compile.cjs hash."""
    for byte in data:
        hash_value ^= byte
        hash_value = (hash_value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return hash_value


def module_hash(fragments):
    """The hash of a module assembled from `fragments` (crate-relative
    paths): their bytes joined with one newline, as `wgsl_module!` joins
    them."""
    source = b"\n".join((CRATE / fragment).read_bytes() for fragment in fragments)
    return f"{fnv1a(0xCBF29CE484222325, source):016x}"


def budget_for(measured):
    """The table's rule: three times the measured time, at least 2000 ms,
    rounded up to the next 100 ms."""
    budget = max(3 * measured, 2000)
    return -(-budget // 100) * 100


class PickVariantTable(unittest.TestCase):
    def setUp(self):
        self.lines = TABLE.read_text(encoding="utf-8").splitlines()

    def header(self, key):
        """`name -> value` of the header lines `# key=name=value`."""
        prefix = f"# {key}="
        return dict(line[len(prefix):].split("=", 1) for line in self.lines
                    if line.startswith(prefix))

    def test_the_rows_were_measured_on_the_current_exact_modules(self):
        hashes = self.header("module_hash")
        fragments = self.header("module_fragments")
        self.assertTrue(hashes)
        self.assertEqual(sorted(hashes), sorted(fragments))
        rows = [line.split(",") for line in self.lines[self.lines.index(COLUMNS) + 1:] if line]
        changed = []
        for name, recorded in sorted(hashes.items()):
            if module_hash(fragments[name].split(";")) != recorded:
                count = sum(1 for fields in rows if f"{fields[0]}/{fields[1]}" == name)
                changed.append(f"{name} ({count} rows)")
        self.assertEqual(
            changed, [],
            f"the exact pick modules {', '.join(changed)} changed since "
            f"{TABLE.relative_to(ROOT)} was measured: regenerate it with `{REGENERATE}`, "
            f"which keeps every other row's time, and measure those rows with "
            f"`{MEASURE} --missing`")

    def test_every_row_has_a_time_within_its_budget_rule(self):
        self.assertIn(COLUMNS, self.lines)
        rows = [line.split(",") for line in self.lines[self.lines.index(COLUMNS) + 1:] if line]
        self.assertTrue(rows)
        for fields in rows:
            self.assertEqual(len(fields), 11, fields)
            name = "/".join(fields[:3] + fields[4:5])
            self.assertTrue(fields[9].isdigit() and fields[10].isdigit(),
                            f"{name} has no measured time or budget: measure it with `{MEASURE}`")
            self.assertEqual(int(fields[10]), budget_for(int(fields[9])),
                             f"{name}: the budget does not follow the rule; re-measure with "
                             f"`{MEASURE}`")


if __name__ == "__main__":
    unittest.main()
