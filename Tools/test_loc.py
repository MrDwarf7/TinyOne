#!/usr/bin/env python3
from __future__ import annotations

import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
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

    def test_letter_index_counts_unicode_letters_per_file_and_in_json(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "src" / "lib.rs"
            source.parent.mkdir()
            source.write_text("a1 \u00e9!\n", encoding="utf-8")

            stat = loc_tool.count_file(root, "src/lib.rs", ".rs", count_letters=True)
            self.assertIsNotNone(stat)
            assert stat is not None
            self.assertEqual(stat.letters, 2)

            payload = loc_tool.build_json_payload(
                [stat],
                extensions=(".rs",),
                largest=None,
                smallest=None,
                warnings=[],
                include_letters=True,
            )
            self.assertEqual(payload["total"]["letters"], 2)
            self.assertEqual(payload["files"][0]["letters"], 2)

    def test_largest_with_letters_limits_both_rankings_to_requested_files(self) -> None:
        stats = [
            loc_tool.FileStat("first_loc.rs", ".rs", 0, 3, 0, 0, letters=3),
            loc_tool.FileStat("second_loc.rs", ".rs", 0, 2, 0, 0, letters=10),
            loc_tool.FileStat("most_letters.rs", ".rs", 0, 1, 0, 0, letters=500),
        ]
        tally = loc_tool.Tally()
        for stat in stats:
            tally.add(stat)
        output = StringIO()
        with redirect_stdout(output):
            loc_tool.emit_table(
                stats,
                {".rs": tally},
                tally,
                [stats[0], stats[1]],
                None,
                [stats[2], stats[1]],
                include_letters=True,
            )

        report = output.getvalue()
        self.assertIn("Largest files (by LoC):", report)
        self.assertIn("Largest files (by letters):", report)
        self.assertNotIn("Letters per file:", report)
        self.assertNotIn("most_letters.rs: 500 LoC", report)
        self.assertNotIn("first_loc.rs: 3 letters", report)
        self.assertIn("most_letters.rs: 500 letters", report)
        self.assertIn("second_loc.rs: 10 letters", report)


if __name__ == "__main__":
    unittest.main()
