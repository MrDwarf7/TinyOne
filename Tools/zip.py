#!/usr/bin/env python3
"""Create zip or tar.gz archives while honoring discovered ignore files."""
from __future__ import annotations

import argparse
import fnmatch
import sys
import tarfile
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, NoReturn, Sequence

ArchiveFormat = Literal["zip", "tar.gz"]
EXIT_OK = 0
EXIT_ERROR = 2
IGNORE_FILENAMES = (".gitignore", ".zipignore")
HELP_EPILOG = """
Examples:
  ./Tools/zip.py TinyOne -o tinyone.zip
  ./Tools/zip.py TinyOne Ralloc -o source.tar.gz --format tar.gz
  ./Tools/zip.py README.rst docs --format zip

Every directory walk loads .gitignore and .zipignore rules from each directory
before reading that directory's children. Nested ignore files override earlier
matches in normal last-match-wins order, including negated patterns such as
!keep.log.
"""


@dataclass(frozen=True)
class IgnoreRule:
    base: Path
    pattern: str
    negated: bool
    directory_only: bool
    anchored: bool
    has_slash: bool

    def matches(self, path: Path, is_dir: bool) -> bool:
        if self.directory_only and not is_dir:
            return False
        try:
            rel = path.relative_to(self.base).as_posix()
        except ValueError:
            return False
        if rel in {"", "."}:
            return False

        pattern = self.pattern
        if self.has_slash:
            if self.anchored:
                return rel == pattern or fnmatch.fnmatchcase(rel, pattern)
            return (
                rel == pattern
                or rel.endswith(f"/{pattern}")
                or fnmatch.fnmatchcase(rel, pattern)
                or any(fnmatch.fnmatchcase(suffix, pattern) for suffix in suffixes(rel))
            )

        return any(fnmatch.fnmatchcase(part, pattern) for part in rel.split("/"))


def suffixes(rel: str) -> list[str]:
    parts = rel.split("/")
    return ["/".join(parts[index:]) for index in range(len(parts))]


def parse_ignore_line(base: Path, raw: str) -> IgnoreRule | None:
    line = raw.rstrip("\n")
    if not line or line.isspace():
        return None
    if line.startswith("#"):
        return None
    if line.startswith("\\#"):
        line = line[1:]

    negated = line.startswith("!")
    if negated:
        line = line[1:]
    if not line:
        return None

    line = line.rstrip()
    directory_only = line.endswith("/")
    if directory_only:
        line = line.rstrip("/")
    anchored = line.startswith("/")
    if anchored:
        line = line.lstrip("/")
    if not line:
        return None

    return IgnoreRule(
        base=base,
        pattern=line,
        negated=negated,
        directory_only=directory_only,
        anchored=anchored,
        has_slash="/" in line,
    )


def load_ignore_rules(directory: Path) -> tuple[IgnoreRule, ...]:
    rules: list[IgnoreRule] = []
    for filename in IGNORE_FILENAMES:
        ignore_file = directory / filename
        if not ignore_file.is_file():
            continue
        with ignore_file.open("r", encoding="utf-8", errors="replace") as handle:
            for raw in handle:
                rule = parse_ignore_line(directory, raw)
                if rule is not None:
                    rules.append(rule)
    return tuple(rules)


def is_ignored(path: Path, is_dir: bool, rules: Sequence[IgnoreRule]) -> bool:
    ignored = False
    for rule in rules:
        if rule.matches(path, is_dir):
            ignored = not rule.negated
    return ignored


def archive_name_for_root(root: Path, path: Path) -> str:
    if path == root:
        return root.name
    return f"{root.name}/{path.relative_to(root).as_posix()}"


def iter_archive_entries(root: Path, inherited_rules: Sequence[IgnoreRule] = ()) -> list[tuple[Path, str]]:
    if root.is_file():
        return [(root, root.name)]
    if not root.is_dir():
        raise ValueError(f"input path does not exist or is not a regular file/directory: {root}")

    entries: list[tuple[Path, str]] = []

    def walk(directory: Path, parent_rules: tuple[IgnoreRule, ...]) -> None:
        rules = (*parent_rules, *load_ignore_rules(directory))
        children = sorted(directory.iterdir(), key=lambda child: child.name)
        for child in children:
            child_is_dir = child.is_dir()
            if is_ignored(child, child_is_dir, rules):
                continue
            if child_is_dir:
                walk(child, rules)
            elif child.is_file():
                entries.append((child, archive_name_for_root(root, child)))

    walk(root, tuple(inherited_rules))
    return entries


def is_relative_to(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def validate_output_path(inputs: Sequence[Path], output: Path) -> None:
    resolved_output = output.resolve(strict=False)
    for input_path in inputs:
        if input_path.is_dir() and is_relative_to(resolved_output, input_path.resolve()):
            raise ValueError(f"output archive is inside input directory: {output}")


def infer_format(output: Path, explicit: str | None) -> ArchiveFormat:
    if explicit in {"zip", "tar.gz"}:
        return explicit  # type: ignore[return-value]
    name = output.name.lower()
    if name.endswith(".tar.gz") or name.endswith(".tgz"):
        return "tar.gz"
    if name.endswith(".zip"):
        return "zip"
    return "zip"


def default_output(inputs: Sequence[Path], archive_format: ArchiveFormat) -> Path:
    stem = inputs[0].name if len(inputs) == 1 else "archive"
    suffix = ".tar.gz" if archive_format == "tar.gz" else ".zip"
    return Path(f"{stem}{suffix}")


def collect_entries(inputs: Sequence[Path]) -> list[tuple[Path, str]]:
    entries: list[tuple[Path, str]] = []
    seen_names: set[str] = set()
    for input_path in inputs:
        for source, arcname in iter_archive_entries(input_path):
            if arcname in seen_names:
                raise ValueError(f"duplicate archive path from inputs: {arcname}")
            seen_names.add(arcname)
            entries.append((source, arcname))
    if not entries:
        raise ValueError("no files to archive after applying ignore rules")
    return entries


def create_archive(paths: Sequence[str | Path], output: str | Path, archive_format: ArchiveFormat | str | None = None) -> Path:
    inputs = [Path(path).expanduser() for path in paths]
    if not inputs:
        raise ValueError("at least one input path is required")
    out = Path(output).expanduser()
    fmt = infer_format(out, archive_format)
    validate_output_path(inputs, out)
    entries = collect_entries(inputs)
    out.parent.mkdir(parents=True, exist_ok=True)

    if fmt == "zip":
        with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for source, arcname in entries:
                archive.write(source, arcname)
    elif fmt == "tar.gz":
        with tarfile.open(out, "w:gz") as archive:
            for source, arcname in entries:
                archive.add(source, arcname=arcname, recursive=False)
    else:
        raise ValueError(f"unsupported archive format: {archive_format}")
    return out


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="zip.py",
        description="Create .zip or .tar.gz archives while honoring discovered .gitignore and .zipignore files.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=HELP_EPILOG,
    )
    parser.add_argument("paths", nargs="+", help="file or directory paths to archive")
    parser.add_argument("-o", "--output", help="archive path to write")
    parser.add_argument("--format", choices=("zip", "tar.gz"), help="archive format; inferred from --output when omitted")
    return parser


def die(message: str) -> NoReturn:
    print(f"zip.py: error: {message}", file=sys.stderr)
    raise SystemExit(EXIT_ERROR)


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    input_paths = [Path(path).expanduser() for path in args.paths]
    fmt = infer_format(Path(args.output), args.format) if args.output else infer_format(Path("archive.zip"), args.format)
    output = Path(args.output).expanduser() if args.output else default_output(input_paths, fmt)
    try:
        archive = create_archive(input_paths, output, fmt)
    except (OSError, ValueError) as exc:
        die(str(exc))
    print(archive)
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main())
