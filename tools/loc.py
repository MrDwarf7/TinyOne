#!/usr/bin/env python3
"""Count lines of code and documentation across the git repository.

Files are discovered with git, so whatever git ignores (via .gitignore,
.git/info/exclude, and the global excludesfile) is ignored here too. Discovery
descends into nested git repositories (embedded repos and submodules), and each
repo's own ignore rules apply within it. Each line is classified as blank,
comment, or code.

Line classification is deliberately simple. For .py, a line is a comment only
if it starts with '#'; triple-quoted strings (including docstrings) count as
code. For .rs/.c/.h, '//' lines and '/* ... */' blocks are comments, the block
tracked line-by-line. A comment marker inside a string literal can be
miscounted; that is accurate enough for counting and keeps the tool simple.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn

SOURCE_EXTENSIONS = (".py", ".rs", ".h", ".c")
DOC_EXTENSIONS = (".md", ".rst")
DEFAULT_EXTENSIONS = SOURCE_EXTENSIONS
DEFAULT_TOP_N = 5
DEFAULT_WARN_LARGE_BYTES = 512 * 1024
EXIT_OK = 0
EXIT_ERROR = 2

HELP_EPILOG = """
Examples:
  ./loc.py                      table of code/comment/blank lines per extension
  ./loc.py --docs --largest 10   count docs and source, then show largest files
  ./loc.py --include rst --json  count only .rst files
  ./loc.py --audit --json        include largest files and large-file warnings
  ./loc.py --largest            top 5 files by code lines
  ./loc.py --largest 10         top 10 files by code lines
  ./loc.py --smallest 3         3 files with the fewest code lines
  ./loc.py --largest --json     JSON output including the largest list

Files are discovered via 'git ls-files --cached --others --exclude-standard',
recursing into nested git repos. Anything git ignores is ignored here too.
Run from inside the repository.
"""


@dataclass
class FileStat:
    path: str
    extension: str
    bytes: int
    code: int
    comment: int
    blank: int
    warnings: tuple[dict[str, object], ...] = ()

    @property
    def total(self) -> int:
        return self.code + self.comment + self.blank


@dataclass
class Tally:
    files: int = 0
    code: int = 0
    comment: int = 0
    blank: int = 0

    @property
    def total(self) -> int:
        return self.code + self.comment + self.blank

    def add(self, stat: FileStat) -> None:
        self.files += 1
        self.code += stat.code
        self.comment += stat.comment
        self.blank += stat.blank


def die(message: str) -> NoReturn:
    print(f"loc.py: error: {message}", file=sys.stderr)
    raise SystemExit(EXIT_ERROR)


def positive_int(value: str) -> int:
    try:
        n = int(value)
    except ValueError:
        raise argparse.ArgumentTypeError(f"{value!r} is not an integer")
    if n <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return n


def parse_byte_size(value: str) -> int:
    units = {
        "b": 1,
        "k": 1024,
        "kb": 1024,
        "m": 1024**2,
        "mb": 1024**2,
        "g": 1024**3,
        "gb": 1024**3,
    }
    raw = value.strip().lower()
    if not raw:
        raise argparse.ArgumentTypeError("size cannot be empty")
    for suffix in sorted(units, key=len, reverse=True):
        if raw.endswith(suffix):
            number = raw[: -len(suffix)].strip()
            multiplier = units[suffix]
            break
    else:
        number = raw
        multiplier = 1
    try:
        size = int(number) * multiplier
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"invalid size: {value}") from exc
    if size <= 0:
        raise argparse.ArgumentTypeError("size must be positive")
    return size


def normalize_extension(value: str) -> str:
    ext = value.strip().lower()
    if not ext:
        raise argparse.ArgumentTypeError("extension cannot be empty")
    if not ext.startswith("."):
        ext = f".{ext}"
    return ext


def selected_extensions(
    include: list[str],
    *,
    include_source: bool,
    include_docs: bool,
) -> tuple[str, ...]:
    if include:
        return tuple(dict.fromkeys(normalize_extension(ext) for ext in include))

    selected: list[str] = []
    if include_source or not include_docs:
        selected.extend(SOURCE_EXTENSIONS)
    if include_docs:
        selected.extend(DOC_EXTENSIONS)
    return tuple(dict.fromkeys(selected))


def ext_of(rel: str, extensions: tuple[str, ...]) -> str:
    for ext in extensions:
        if rel.endswith(ext):
            return ext
    return ""


def run_git(args: list[str], root: Path | None = None) -> subprocess.CompletedProcess[bytes]:
    cmd = ["git"]
    if root is not None:
        cmd += ["-C", str(root)]
    cmd += args
    try:
        return subprocess.run(cmd, capture_output=True)
    except FileNotFoundError:
        die("git executable not found on PATH")


def git_toplevel() -> Path:
    result = run_git(["rev-parse", "--show-toplevel"])
    if result.returncode != 0:
        die("not inside a git repository (git rev-parse --show-toplevel failed)")
    return Path(os.fsdecode(result.stdout).strip())


def list_repo_entries(repo_root: Path) -> list[str]:
    result = run_git(
        ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], root=repo_root
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", "replace").strip()
        print(
            f"loc.py: warning: skipping repo {repo_root}: {detail or 'git ls-files failed'}",
            file=sys.stderr,
        )
        return []
    return [os.fsdecode(chunk) for chunk in result.stdout.split(b"\0") if chunk]


def discover_files(top_root: Path, extensions: tuple[str, ...]) -> list[str]:
    """Enumerate target files under top_root, descending into nested repos.

    git ls-files reports a nested repository (embedded repo or submodule) as a
    single entry rather than its contents, so each such boundary is recursed
    into with its own ls-files. Paths are accumulated relative to top_root.
    """
    rels: list[str] = []
    seen_repos: set[Path] = set()

    def walk(repo_root: Path, prefix: str) -> None:
        real = repo_root.resolve()
        if real in seen_repos:  # guard against symlinked filesystem cycles
            return
        seen_repos.add(real)
        for entry in list_repo_entries(repo_root):
            name = entry.rstrip("/")
            candidate = repo_root / name
            if (candidate / ".git").exists():  # nested repo boundary
                walk(candidate, f"{prefix}{name}/")
                continue
            rel = f"{prefix}{entry}"
            if rel.endswith(extensions) and (top_root / rel).is_file():
                rels.append(rel)

    walk(top_root, "")
    return rels


def classify_python(lines: list[str]) -> tuple[int, int, int]:
    code = comment = blank = 0
    for raw in lines:
        stripped = raw.strip()
        if not stripped:
            blank += 1
        elif stripped.startswith("#"):
            comment += 1
        else:
            code += 1
    return code, comment, blank


def classify_c_like(lines: list[str]) -> tuple[int, int, int]:
    code = comment = blank = 0
    in_block = False
    for raw in lines:
        stripped = raw.strip()
        if not stripped:
            if in_block:
                comment += 1
            else:
                blank += 1
            continue
        if in_block:
            comment += 1
            if "*/" in stripped:
                in_block = False
            continue
        if stripped.startswith("//"):
            comment += 1
            continue
        if stripped.startswith("/*"):
            comment += 1
            if "*/" not in stripped[2:]:
                in_block = True
            continue
        code += 1
    return code, comment, blank


def classify_plain_text(lines: list[str]) -> tuple[int, int, int]:
    code = comment = blank = 0
    for raw in lines:
        if raw.strip():
            code += 1
        else:
            blank += 1
    return code, comment, blank


def count_file(
    root: Path,
    rel: str,
    ext: str,
    *,
    warn_large_bytes: int = DEFAULT_WARN_LARGE_BYTES,
) -> FileStat | None:
    full = root / rel
    try:
        size = full.stat().st_size
        text = full.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        print(f"loc.py: warning: skipping {rel}: {exc}", file=sys.stderr)
        return None
    lines = text.splitlines()
    if ext == ".py":
        code, comment, blank = classify_python(lines)
    elif ext in {".rs", ".c", ".h"}:
        code, comment, blank = classify_c_like(lines)
    else:
        code, comment, blank = classify_plain_text(lines)

    warnings: list[dict[str, object]] = []
    if size >= warn_large_bytes:
        warnings.append(
            {
                "kind": "large_file",
                "path": rel,
                "bytes": size,
                "threshold_bytes": warn_large_bytes,
            }
        )
    return FileStat(rel, ext, size, code, comment, blank, tuple(warnings))


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="loc.py",
        description="Count source and documentation lines across the git repository.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=HELP_EPILOG,
    )
    parser.add_argument(
        "--include",
        action="append",
        default=[],
        metavar="EXT",
        help="Only count this extension. Repeatable. Overrides --source/--docs defaults.",
    )
    parser.add_argument(
        "--source",
        action="store_true",
        help="Count source extensions (.py, .rs, .h, .c). This is the default when no filter is set.",
    )
    parser.add_argument(
        "--docs",
        action="store_true",
        help="Count documentation extensions (.md, .rst). Combine with --source to count both.",
    )
    parser.add_argument(
        "--audit",
        action="store_true",
        help=f"Shortcut for --largest {DEFAULT_TOP_N} plus large-file warnings.",
    )
    parser.add_argument(
        "--warn-large",
        type=parse_byte_size,
        default=DEFAULT_WARN_LARGE_BYTES,
        metavar="SIZE",
        help="Warn when a counted file is at least SIZE bytes. Accepts kb/mb suffixes.",
    )
    parser.add_argument(
        "--largest",
        nargs="?",
        type=positive_int,
        const=DEFAULT_TOP_N,
        default=None,
        metavar="N",
        help=f"list the N files with the most code lines (bare flag: N={DEFAULT_TOP_N})",
    )
    parser.add_argument(
        "--smallest",
        nargs="?",
        type=positive_int,
        const=DEFAULT_TOP_N,
        default=None,
        metavar="N",
        help=f"list the N files with the fewest code lines (bare flag: N={DEFAULT_TOP_N})",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit machine-readable JSON instead of a table",
    )
    return parser


def emit_table(
    tallies: dict[str, Tally],
    grand: Tally,
    largest: list[FileStat] | None,
    smallest: list[FileStat] | None,
) -> None:
    headers = ("EXT", "FILES", "CODE", "COMMENT", "BLANK", "TOTAL")
    rows = [
        (ext, t.files, t.code, t.comment, t.blank, t.total)
        for ext, t in tallies.items()
    ]
    total_row = ("TOTAL", grand.files, grand.code, grand.comment, grand.blank, grand.total)

    str_rows = [tuple(str(c) for c in row) for row in (*rows, total_row)]
    widths = [len(h) for h in headers]
    for row in str_rows:
        for i, cell in enumerate(row):
            widths[i] = max(widths[i], len(cell))

    def fmt(row: tuple[str, ...]) -> str:
        cells = [
            cell.ljust(widths[i]) if i == 0 else cell.rjust(widths[i])
            for i, cell in enumerate(row)
        ]
        return "  ".join(cells)

    out = [fmt(headers)]
    out.extend(fmt(row) for row in str_rows[:-1])
    out.append("  ".join("-" * w for w in widths))
    out.append(fmt(str_rows[-1]))
    print("\n".join(out))

    for title, group in (("Largest", largest), ("Smallest", smallest)):
        if group is None:
            continue
        print()
        print(f"{title} files (by LoC):")
        if not group:
            print("  (none)")
        for stat in group:
            print(f"  {stat.path}: {stat.code} LoC")


def emit_json(
    stats: list[FileStat],
    tallies: dict[str, Tally],
    grand: Tally,
    extensions: tuple[str, ...],
    largest: list[FileStat] | None,
    smallest: list[FileStat] | None,
    warnings: list[dict[str, object]],
) -> None:
    payload = build_json_payload(
        stats,
        extensions=extensions,
        largest=largest,
        smallest=smallest,
        warnings=warnings,
    )
    print(json.dumps(payload, indent=2))


def build_json_payload(
    stats: list[FileStat],
    *,
    extensions: tuple[str, ...],
    largest: list[FileStat] | None,
    smallest: list[FileStat] | None,
    warnings: list[dict[str, object]],
) -> dict[str, object]:
    tallies: dict[str, Tally] = {ext: Tally() for ext in extensions}
    grand = Tally()
    for stat in stats:
        tallies.setdefault(stat.extension, Tally()).add(stat)
        grand.add(stat)

    def tally_dict(t: Tally) -> dict[str, int]:
        return {
            "files": t.files,
            "code": t.code,
            "comment": t.comment,
            "blank": t.blank,
            "total": t.total,
        }

    def file_dict(s: FileStat) -> dict[str, object]:
        return {
            "path": s.path,
            "extension": s.extension,
            "bytes": s.bytes,
            "code": s.code,
            "comment": s.comment,
            "blank": s.blank,
            "total": s.total,
        }

    payload: dict[str, object] = {
        "extensions": list(extensions),
        "by_extension": {ext: tally_dict(t) for ext, t in tallies.items()},
        "total": tally_dict(grand),
        "files": [file_dict(s) for s in sorted(stats, key=lambda s: (-s.code, s.path))],
        "warnings": warnings,
    }
    if largest is not None:
        payload["largest"] = [file_dict(s) for s in largest]
    if smallest is not None:
        payload["smallest"] = [file_dict(s) for s in smallest]
    return payload


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    root = git_toplevel()
    extensions = selected_extensions(
        args.include,
        include_source=args.source,
        include_docs=args.docs,
    )
    rels = discover_files(root, extensions)

    stats: list[FileStat] = []
    tallies: dict[str, Tally] = {ext: Tally() for ext in extensions}
    warnings: list[dict[str, object]] = []
    for rel in rels:
        ext = ext_of(rel, extensions)
        stat = count_file(root, rel, ext, warn_large_bytes=args.warn_large)
        if stat is None:
            continue
        stats.append(stat)
        tallies[ext].add(stat)
        warnings.extend(stat.warnings)

    grand = Tally()
    for stat in stats:
        grand.add(stat)

    largest = (
        sorted(stats, key=lambda s: (-s.code, s.path))[: (args.largest or DEFAULT_TOP_N)]
        if args.largest is not None or args.audit
        else None
    )
    smallest = (
        sorted(stats, key=lambda s: (s.code, s.path))[: args.smallest]
        if args.smallest is not None
        else None
    )

    if args.json:
        emit_json(stats, tallies, grand, extensions, largest, smallest, warnings)
    else:
        emit_table(tallies, grand, largest, smallest)
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main())
