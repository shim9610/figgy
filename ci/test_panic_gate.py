import json
import unittest

import verify


def message(code, file_name, line, expansion=None):
    span = {"is_primary": True, "file_name": file_name, "line_start": line,
            "expansion": expansion}
    return json.dumps({"reason": "compiler-message",
                       "message": {"code": {"code": code}, "spans": [span]}})


SOURCE = {
    "src/a.rs": [
        "fn f() {",
        "    x.unwrap();",
        "}",
        "const _: () =",
        "    assert!(N == 4);",
        "fn g() {",
        "    assert!(ok);",
        "}",
    ],
}


def read(name):
    return SOURCE[name]


class PanicGateTest(unittest.TestCase):
    def test_runtime_panics_are_reported(self):
        found = verify.panic_gate_findings([
            message("clippy::unwrap_used", "src/a.rs", 2),
            message("clippy::disallowed_macros", "src/a.rs", 7),
        ], read)
        self.assertEqual(found, ["src/a.rs:2 clippy::unwrap_used",
                                 "src/a.rs:7 clippy::disallowed_macros"])

    def test_const_items_and_derives_are_build_time_checks(self):
        derive = {"macro_decl_name": "#[derive(bytemuck::Pod)]",
                  "span": {"file_name": "src/a.rs", "line_start": 1, "expansion": None}}
        found = verify.panic_gate_findings([
            message("clippy::disallowed_macros", "src/a.rs", 5),
            message("clippy::disallowed_macros", "/rustc/x/core/src/macros/mod.rs", 9, derive),
        ], read)
        self.assertEqual(found, [])

    def test_core_macro_spans_point_at_the_call_site(self):
        call = {"macro_decl_name": "debug_assert_eq!",
                "span": {"file_name": "src/a.rs", "line_start": 7, "expansion": None}}
        found = verify.panic_gate_findings([
            message("clippy::disallowed_macros", "/rustc/x/core/src/macros/mod.rs", 9, call),
        ], read)
        self.assertEqual(found, ["src/a.rs:7 clippy::disallowed_macros"])

    def test_non_clippy_messages_are_ignored(self):
        self.assertEqual(verify.panic_gate_findings(["not json", json.dumps({"reason": "x"})],
                                                    read), [])


if __name__ == "__main__":
    unittest.main()
