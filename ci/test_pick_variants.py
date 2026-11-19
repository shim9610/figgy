"""The exact pick variant table (#363) belongs to the current shaders.

`crates/renderer/tests/support/pick_variants.csv` lists every exact pick
pipeline a chart can reach with its measured SwiftShader compile time. It
records a hash over every WGSL file of the renderer; any shader change
makes this test fail until the table is regenerated and re-measured. The
rows themselves are checked against the picker by the renderer's
`pick_variant_table_matches_the_picker` test.
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


def wgsl_hash():
    """Each WGSL file under the renderer's `src/`, by its crate-relative path
    with `/` separators in path order: the path, a NUL, the bytes, a NUL."""
    hash_value = 0xCBF29CE484222325
    for path in sorted((CRATE / "src").rglob("*.wgsl"),
                       key=lambda p: p.relative_to(CRATE).as_posix()):
        hash_value = fnv1a(hash_value, path.relative_to(CRATE).as_posix().encode())
        hash_value = fnv1a(hash_value, b"\0")
        hash_value = fnv1a(hash_value, path.read_bytes())
        hash_value = fnv1a(hash_value, b"\0")
    return f"{hash_value:016x}"


def budget_for(measured):
    """The table's rule: three times the measured time, at least 2000 ms,
    rounded up to the next 100 ms."""
    budget = max(3 * measured, 2000)
    return -(-budget // 100) * 100


class PickVariantTable(unittest.TestCase):
    def setUp(self):
        self.lines = TABLE.read_text(encoding="utf-8").splitlines()

    def test_the_table_was_measured_on_the_current_shaders(self):
        recorded = [line.split("=", 1)[1] for line in self.lines
                    if line.startswith("# wgsl_hash=")]
        self.assertEqual(
            recorded, [wgsl_hash()],
            f"the WGSL changed since {TABLE.relative_to(ROOT)} was measured: regenerate it "
            f"with `{REGENERATE}` and re-measure every row with `{MEASURE}`")

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
