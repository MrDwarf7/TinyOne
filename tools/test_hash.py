#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import hash as hash_tool


class HashToolTests(unittest.TestCase):
    def test_default_excludes_match_current_layout_targets(self) -> None:
        excludes = hash_tool.defaulted_exclude_patterns((), use_defaults=True)

        self.assertIn("TinyOne/target", excludes)
        self.assertIn("Ralloc/target", excludes)
        self.assertNotIn("Rust/target", excludes)

    def test_tree_hash_skips_current_layout_build_outputs_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "TinyOne" / "src").mkdir(parents=True)
            (root / "TinyOne" / "target" / "debug").mkdir(parents=True)
            (root / "Ralloc" / "target").mkdir(parents=True)
            (root / "TinyOne" / "src" / "lib.rs").write_text("pub fn ok() {}\n", encoding="utf-8")
            (root / "TinyOne" / "target" / "debug" / "tinylang").write_text("drop\n", encoding="utf-8")
            (root / "Ralloc" / "target" / "lib.a").write_text("drop\n", encoding="utf-8")

            result = hash_tool.build_tree_result(
                root,
                "sha256",
                hash_tool.DEFAULT_CHUNK_SIZE,
                hash_tool.normalize_suffixes(()),
                hash_tool.defaulted_exclude_patterns((), use_defaults=True),
                "error",
                None,
                True,
            )

            self.assertEqual([file.path for file in result.files or ()], ["TinyOne/src/lib.rs"])

    def test_manifest_check_reports_all_entries_without_aborting_on_missing_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            present = root / "present.txt"
            missing = root / "missing.txt"
            present.write_text("release\n", encoding="utf-8")
            good = hash_tool.hash_file(present, "sha256", hash_tool.DEFAULT_CHUNK_SIZE)
            manifest = root / "manifest.json"
            manifest.write_text(
                json.dumps(
                    [
                        {
                            "mode": "file",
                            "name": "present",
                            "path": str(present),
                            "algorithm": "sha256",
                            "digest": good,
                        },
                        {
                            "mode": "file",
                            "name": "missing",
                            "path": str(missing),
                            "algorithm": "sha256",
                            "digest": good,
                        },
                    ]
                ),
                encoding="utf-8",
            )

            results = hash_tool.verify_manifest(manifest, hash_tool.DEFAULT_CHUNK_SIZE)

            self.assertEqual([result.ok for result in results], [True, False])
            self.assertIn("not a file", results[1].message)


if __name__ == "__main__":
    unittest.main()
