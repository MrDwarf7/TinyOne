#!/usr/bin/env python3
from __future__ import annotations

import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import zip as zip_tool


class ArchiveToolTests(unittest.TestCase):
    def test_zip_respects_root_and_nested_gitignore_rules(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "project"
            (root / "src" / "keep").mkdir(parents=True)
            (root / "src" / "ignored").mkdir()
            (root / ".gitignore").write_text("*.log\nbuild/\n", encoding="utf-8")
            (root / "src" / ".gitignore").write_text(
                "ignored/\n!important.log\n", encoding="utf-8"
            )
            (root / "app.py").write_text("print('ok')\n", encoding="utf-8")
            (root / "debug.log").write_text("drop\n", encoding="utf-8")
            (root / "build" / "artifact.txt").parent.mkdir()
            (root / "build" / "artifact.txt").write_text("drop\n", encoding="utf-8")
            (root / "src" / "keep" / "main.rs").write_text("fn main() {}\n", encoding="utf-8")
            (root / "src" / "ignored" / "secret.txt").write_text("drop\n", encoding="utf-8")
            (root / "src" / "important.log").write_text("keep\n", encoding="utf-8")
            out = Path(tmp) / "bundle.zip"

            zip_tool.create_archive([root], out, "zip")

            with zipfile.ZipFile(out) as archive:
                names = sorted(archive.namelist())

            self.assertEqual(
                names,
                [
                    "project/.gitignore",
                    "project/app.py",
                    "project/src/.gitignore",
                    "project/src/important.log",
                    "project/src/keep/main.rs",
                ],
            )

    def test_zipignore_rules_are_loaded_alongside_gitignore_rules(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "project"
            (root / "nested").mkdir(parents=True)
            (root / ".gitignore").write_text("*.tmp\n", encoding="utf-8")
            (root / ".zipignore").write_text("private/\n*.secret\n", encoding="utf-8")
            (root / "nested" / ".zipignore").write_text("*.generated\n", encoding="utf-8")
            (root / "keep.txt").write_text("keep\n", encoding="utf-8")
            (root / "scratch.tmp").write_text("drop\n", encoding="utf-8")
            (root / "token.secret").write_text("drop\n", encoding="utf-8")
            (root / "private" / "key.txt").parent.mkdir()
            (root / "private" / "key.txt").write_text("drop\n", encoding="utf-8")
            (root / "nested" / "view.txt").write_text("keep\n", encoding="utf-8")
            (root / "nested" / "view.generated").write_text("drop\n", encoding="utf-8")
            out = Path(tmp) / "bundle.zip"

            zip_tool.create_archive([root], out, "zip")

            with zipfile.ZipFile(out) as archive:
                names = sorted(archive.namelist())

            self.assertEqual(
                names,
                [
                    "project/.gitignore",
                    "project/.zipignore",
                    "project/keep.txt",
                    "project/nested/.zipignore",
                    "project/nested/view.txt",
                ],
            )

    def test_tar_gz_supports_multiple_inputs_and_single_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            first = base / "first.txt"
            second_dir = base / "second"
            first.write_text("one\n", encoding="utf-8")
            second_dir.mkdir()
            (second_dir / "two.txt").write_text("two\n", encoding="utf-8")
            out = base / "bundle.tar.gz"

            zip_tool.create_archive([first, second_dir], out, "tar.gz")

            with tarfile.open(out, "r:gz") as archive:
                names = sorted(member.name for member in archive.getmembers() if member.isfile())

            self.assertEqual(names, ["first.txt", "second/two.txt"])

    def test_output_name_infers_tar_gz_format_without_format_flag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            source = base / "source"
            source.mkdir()
            (source / "file.txt").write_text("data\n", encoding="utf-8")
            out = base / "named-output.tar.gz"

            self.assertEqual(zip_tool.infer_format(out, None), "tar.gz")
            zip_tool.create_archive([source], out)

            with tarfile.open(out, "r:gz") as archive:
                names = sorted(member.name for member in archive.getmembers() if member.isfile())

            self.assertEqual(names, ["source/file.txt"])

    def test_refuses_output_inside_scraped_directory_when_not_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "project"
            root.mkdir()
            (root / "file.txt").write_text("data\n", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "output archive is inside input directory"):
                zip_tool.create_archive([root], root / "bundle.zip", "zip")


if __name__ == "__main__":
    unittest.main()
