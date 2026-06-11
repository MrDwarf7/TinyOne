#!/usr/bin/env python3
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import loc as loc_tool


class LocToolTests(unittest.TestCase):
    def test_extension_selection_includes_docs_when_requested(self) -> None:
        extensions = loc_tool.selected_extensions([], include_source=False, include_docs=True)

        self.assertEqual(extensions, (".md", ".rst"))
        self.assertEqual(
            loc_tool.selected_extensions(["rst"], include_source=False, include_docs=True),
            (".rst",),
        )

    def test_audit_payload_contains_largest_files_and_warnings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            small = root / "src" / "lib.rs"
            large = root / "README.rst"
            small.parent.mkdir()
            small.write_text("fn main() {}\n", encoding="utf-8")
            large.write_text("\n".join(f"line {i}" for i in range(12)) + "\n", encoding="utf-8")

            stats = [
                loc_tool.count_file(root, "src/lib.rs", ".rs", warn_large_bytes=100),
                loc_tool.count_file(root, "README.rst", ".rst", warn_large_bytes=20),
            ]
            payload = loc_tool.build_json_payload(
                [stat for stat in stats if stat is not None],
                extensions=(".rs", ".rst"),
                largest=[stat for stat in stats if stat is not None],
                smallest=None,
                warnings=[warning for stat in stats if stat for warning in stat.warnings],
            )

            self.assertEqual(payload["files"][0]["path"], "README.rst")
            self.assertEqual(payload["files"][0]["extension"], ".rst")
            self.assertIn("bytes", payload["files"][0])
            self.assertEqual(payload["warnings"][0]["path"], "README.rst")
            self.assertEqual(payload["warnings"][0]["kind"], "large_file")


if __name__ == "__main__":
    unittest.main()
