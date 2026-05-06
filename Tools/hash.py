#!/usr/bin/env python3
from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import os
from collections.abc import Iterable, Sequence
from dataclasses import asdict, dataclass
from pathlib import Path
from textwrap import dedent
from typing import Literal

HashAlgorithm = Literal["sha256", "sha512", "blake2b", "md5"]
OutputFormat = Literal["plain", "json", "release"]
SymlinkPolicy = Literal["follow", "skip", "error"]
Mode = Literal["file", "tree"]

DEFAULT_CHUNK_SIZE = 1024 * 1024
DEFAULT_ALGORITHM: HashAlgorithm = "sha256"
EXIT_OK = 0
EXIT_VERIFY_FAILED = 1
EXIT_ERROR = 2
TREE_FORMAT_VERSION = b"hash-tool-tree-v1\0"
DEFAULT_EXCLUDE_PATTERNS = (
    ".git",
    "Rust/target",
    "__pycache__",
    ".mypy_cache",
    ".agents",
    ".codex",
)
HELP_EPILOG = """
Examples:
  ./hash.py README.md
  ./hash.py -a sha256 --format release --name TinyOne Python/main.py
  ./hash.py --tree . --exclude manifest.json --symlinks skip --format json > manifest.json
  ./hash.py --tree . --format json --list-files > manifest.json
  ./hash.py --expected <digest> README.md
  ./hash.py --check manifest.json

Modes:
  File mode is the default. Pass one or more file paths.
  Tree mode uses --tree DIR and hashes every included file into one stable digest.
  Verify mode uses --expected DIGEST FILE or --check manifest.json.

Output formats:
  plain    "<digest>  <path>" lines, like common checksum tools.
  json     machine-readable HashResult records.
  release  markdown bullets for release notes.
  Use --list-files with --tree to include per-file path, size, algorithm, and digest details.

Exit codes:
  0  success or all verification entries passed.
  1  at least one verification entry failed.
  2  usage, manifest, file, or hashing error.
"""


@dataclass(frozen=True)
class TreeFileHash:
    path: str
    size: int
    algorithm: HashAlgorithm
    digest: str


@dataclass(frozen=True)
class HashResult:
    mode: Mode
    name: str
    path: str
    algorithm: HashAlgorithm
    digest: str
    files_hashed: int | None = None
    include_suffixes: tuple[str, ...] | None = None
    exclude_patterns: tuple[str, ...] | None = None
    symlink_policy: SymlinkPolicy | None = None
    files: tuple[TreeFileHash, ...] | None = None


@dataclass(frozen=True)
class VerificationResult:
    mode: Mode
    path: str
    algorithm: HashAlgorithm
    expected: str
    actual: str | None
    ok: bool
    message: str = ""
    expected_files_hashed: int | None = None
    actual_files_hashed: int | None = None


def normalize_algorithm(value: str) -> HashAlgorithm:
    match value.lower():
        case "sha256":
            return "sha256"
        case "sha512":
            return "sha512"
        case "blake2b":
            return "blake2b"
        case "md5":
            return "md5"
        case _:
            raise argparse.ArgumentTypeError(f"unsupported algorithm: {value}")


def new_hasher(algorithm: HashAlgorithm) -> hashlib._Hash:
    match algorithm:
        case "sha256":
            return hashlib.sha256()
        case "sha512":
            return hashlib.sha512()
        case "blake2b":
            return hashlib.blake2b()
        case "md5":
            return hashlib.md5(usedforsecurity=False)


def parse_chunk_size(value: str) -> int:
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
        raise argparse.ArgumentTypeError("chunk size cannot be empty")

    for suffix in sorted(units, key=len, reverse=True):
        if raw.endswith(suffix):
            number = raw[: -len(suffix)].strip()
            multiplier = units[suffix]
            break
    else:
        number = raw
        multiplier = 1

    if not number:
        raise argparse.ArgumentTypeError(f"invalid chunk size: {value}")

    try:
        parsed = int(number) * multiplier
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"invalid chunk size: {value}") from exc

    if parsed <= 0:
        raise argparse.ArgumentTypeError("chunk size must be greater than zero")

    return parsed


def normalize_suffixes(values: Sequence[str]) -> frozenset[str]:
    suffixes: set[str] = set()

    for value in values:
        suffix = value.strip().lower()
        if not suffix:
            raise argparse.ArgumentTypeError("include suffix cannot be empty")
        if not suffix.startswith("."):
            suffix = f".{suffix}"
        suffixes.add(suffix)

    return frozenset(suffixes)


def normalize_exclude_patterns(values: Sequence[str]) -> tuple[str, ...]:
    patterns: list[str] = []

    for value in values:
        pattern = value.strip().replace("\\", "/").strip("/")
        if not pattern:
            raise argparse.ArgumentTypeError("exclude pattern cannot be empty")
        patterns.append(pattern)

    return tuple(dict.fromkeys(patterns))


def normalize_digest(value: str) -> str:
    digest = value.strip().lower()
    if not digest:
        raise ValueError("digest cannot be empty")
    try:
        bytes.fromhex(digest)
    except ValueError as exc:
        raise ValueError(f"digest must be hexadecimal: {value}") from exc
    return digest


def normalize_symlink_policy(value: object) -> SymlinkPolicy:
    if value == "follow":
        return "follow"
    if value == "skip":
        return "skip"
    if value == "error":
        return "error"
    raise ValueError(f"unsupported symlink policy: {value}")


def defaulted_exclude_patterns(
    user_patterns: Sequence[str],
    *,
    use_defaults: bool,
) -> tuple[str, ...]:
    base = DEFAULT_EXCLUDE_PATTERNS if use_defaults else ()
    return normalize_exclude_patterns((*base, *user_patterns))


def encode_u64(value: int) -> bytes:
    if value < 0:
        raise ValueError("cannot encode negative integer")
    return value.to_bytes(8, "big")


def update_framed(hasher: hashlib._Hash, payload: bytes) -> None:
    hasher.update(encode_u64(len(payload)))
    hasher.update(payload)


def hash_file(path: Path, algorithm: HashAlgorithm, chunk_size: int) -> str:
    if path.is_symlink():
        # File mode follows Path.open() behavior by default at the CLI layer.
        # The check is kept here only to avoid hiding unexpected broken links.
        if not path.exists():
            raise FileNotFoundError(path)

    hasher = new_hasher(algorithm)

    with path.open("rb") as file:
        while True:
            chunk = file.read(chunk_size)
            if not chunk:
                break
            hasher.update(chunk)

    return hasher.hexdigest()


def should_include_file(path: Path, include_suffixes: frozenset[str]) -> bool:
    if not include_suffixes:
        return True
    return path.suffix.lower() in include_suffixes


def path_matches_pattern(relative_path: str, pattern: str) -> bool:
    components = relative_path.split("/")
    if "/" not in pattern:
        return any(fnmatch.fnmatchcase(component, pattern) for component in components)
    return (
        relative_path == pattern
        or relative_path.startswith(f"{pattern}/")
        or fnmatch.fnmatchcase(relative_path, pattern)
    )


def should_exclude_path(root: Path, path: Path, exclude_patterns: Sequence[str]) -> bool:
    if not exclude_patterns:
        return False
    relative_path = path.relative_to(root).as_posix()
    return any(path_matches_pattern(relative_path, pattern) for pattern in exclude_patterns)


def iter_tree_files(
    root: Path,
    include_suffixes: frozenset[str],
    exclude_patterns: Sequence[str],
    symlinks: SymlinkPolicy,
) -> Iterable[Path]:
    for dirpath, dirnames, filenames in os.walk(root, followlinks=(symlinks == "follow")):
        current_dir = Path(dirpath)

        retained_dirnames: list[str] = []
        for dirname in dirnames:
            child = current_dir / dirname
            if should_exclude_path(root, child, exclude_patterns):
                continue
            if child.is_symlink() and symlinks in {"skip", "error"}:
                if symlinks == "error":
                    raise RuntimeError(f"symlinked directory encountered: {child}")
                continue
            retained_dirnames.append(dirname)
        dirnames[:] = retained_dirnames

        for filename in filenames:
            path = current_dir / filename

            if should_exclude_path(root, path, exclude_patterns):
                continue
            if path.is_symlink():
                if symlinks == "error":
                    raise RuntimeError(f"symlinked file encountered: {path}")
                if symlinks == "skip":
                    continue

            if path.is_file() and should_include_file(path, include_suffixes):
                yield path


def hash_tree_manifest(
    root: Path,
    algorithm: HashAlgorithm,
    chunk_size: int,
    include_suffixes: frozenset[str],
    exclude_patterns: Sequence[str],
    symlinks: SymlinkPolicy,
) -> tuple[str, tuple[TreeFileHash, ...]]:
    files = sorted(
        iter_tree_files(root, include_suffixes, exclude_patterns, symlinks),
        key=lambda p: p.relative_to(root).as_posix(),
    )

    if not files:
        suffix_msg = "" if not include_suffixes else f" matching {sorted(include_suffixes)}"
        raise ValueError(f"no{suffix_msg} files under {root}")

    hasher = new_hasher(algorithm)
    hasher.update(TREE_FORMAT_VERSION)
    update_framed(hasher, algorithm.encode("ascii"))
    hasher.update(encode_u64(len(files)))

    file_hashes: list[TreeFileHash] = []
    for path in files:
        relative_path = path.relative_to(root).as_posix()
        rel = relative_path.encode("utf-8")
        digest_hex = hash_file(path, algorithm, chunk_size)
        digest_bytes = bytes.fromhex(digest_hex)
        file_hashes.append(
            TreeFileHash(
                path=relative_path,
                size=path.stat().st_size,
                algorithm=algorithm,
                digest=digest_hex,
            )
        )

        update_framed(hasher, rel)
        update_framed(hasher, digest_bytes)

    return hasher.hexdigest(), tuple(file_hashes)


def hash_tree(
    root: Path,
    algorithm: HashAlgorithm,
    chunk_size: int,
    include_suffixes: frozenset[str],
    exclude_patterns: Sequence[str],
    symlinks: SymlinkPolicy,
) -> tuple[str, int]:
    digest, files = hash_tree_manifest(
        root,
        algorithm,
        chunk_size,
        include_suffixes,
        exclude_patterns,
        symlinks,
    )
    return digest, len(files)


def name_for_path(path: Path, explicit_name: str | None) -> str:
    if explicit_name is not None:
        stripped = explicit_name.strip()
        if not stripped:
            raise ValueError("name cannot be empty")
        return stripped
    return path.name or path.resolve().name


def build_file_results(
    paths: Sequence[Path],
    algorithm: HashAlgorithm,
    chunk_size: int,
    name: str | None,
) -> list[HashResult]:
    if name is not None and len(paths) != 1:
        raise ValueError("--name can only be used with one file path")

    results: list[HashResult] = []
    for path in paths:
        if not path.is_file():
            raise ValueError(f"not a file: {path}")

        results.append(
            HashResult(
                mode="file",
                name=name_for_path(path, name),
                path=str(path),
                algorithm=algorithm,
                digest=hash_file(path, algorithm, chunk_size),
            )
        )

    return results


def build_tree_result(
    root: Path,
    algorithm: HashAlgorithm,
    chunk_size: int,
    include_suffixes: frozenset[str],
    exclude_patterns: Sequence[str],
    symlinks: SymlinkPolicy,
    name: str | None,
    list_files: bool,
) -> HashResult:
    if not root.is_dir():
        raise ValueError(f"not a directory: {root}")

    digest, files = hash_tree_manifest(
        root,
        algorithm,
        chunk_size,
        include_suffixes,
        exclude_patterns,
        symlinks,
    )
    return HashResult(
        mode="tree",
        name=name_for_path(root, name),
        path=str(root),
        algorithm=algorithm,
        digest=digest,
        files_hashed=len(files),
        include_suffixes=tuple(sorted(include_suffixes)),
        exclude_patterns=tuple(exclude_patterns),
        symlink_policy=symlinks,
        files=files if list_files else None,
    )


def render_plain(results: Sequence[HashResult]) -> str:
    lines: list[str] = []
    for result in results:
        lines.append(f"{result.digest}  {result.path}")
        if result.files is not None:
            for file_hash in result.files:
                lines.append(
                    f"{file_hash.algorithm}  {file_hash.digest}  {file_hash.size}  {file_hash.path}"
                )
    return "\n".join(lines)


def hash_result_to_dict(result: HashResult) -> dict[str, object]:
    payload = asdict(result)
    if result.files is None:
        payload.pop("files")
    return payload


def render_json(results: Sequence[HashResult]) -> str:
    payload = [hash_result_to_dict(result) for result in results]
    return json.dumps(payload, indent=2, sort_keys=True)


def file_count_label(count: int) -> str:
    suffix = "file" if count == 1 else "files"
    return f"{count} {suffix}"


def render_release(results: Sequence[HashResult]) -> str:
    lines: list[str] = []
    for result in results:
        label = result.algorithm.upper()
        if result.mode == "tree" and result.files_hashed is not None:
            label = f"{label}, {file_count_label(result.files_hashed)}"
        lines.append(f"- {result.name} Hash ({label}): `{result.digest}`")
        if result.files is not None:
            for file_hash in result.files:
                detail = (
                    f"  - {file_hash.path} "
                    f"({file_hash.size} bytes, {file_hash.algorithm.upper()}): "
                    f"`{file_hash.digest}`"
                )
                lines.append(detail)
    return "\n".join(lines)


def render(results: Sequence[HashResult], output_format: OutputFormat) -> str:
    match output_format:
        case "plain":
            return render_plain(results)
        case "json":
            return render_json(results)
        case "release":
            return render_release(results)


def render_verification_plain(results: Sequence[VerificationResult]) -> str:
    lines: list[str] = []
    for result in results:
        if result.ok:
            lines.append(f"OK  {result.path}")
            continue
        detail = result.message
        if result.actual is not None:
            detail = f"expected {result.expected} got {result.actual}"
        lines.append(f"FAIL  {result.path}  {detail}")
    return "\n".join(lines)


def render_verification(
    results: Sequence[VerificationResult],
    output_format: OutputFormat,
) -> str:
    match output_format:
        case "plain":
            return render_verification_plain(results)
        case "json":
            payload = [asdict(result) for result in results]
            return json.dumps(payload, indent=2, sort_keys=True)
        case "release":
            raise ValueError("--format release is not valid in verify mode")


def verify_file_entry(
    path: Path,
    *,
    display_path: str,
    algorithm: HashAlgorithm,
    expected_digest: str,
    chunk_size: int,
) -> VerificationResult:
    actual_digest = hash_file(path, algorithm, chunk_size)
    return VerificationResult(
        mode="file",
        path=display_path,
        algorithm=algorithm,
        expected=expected_digest,
        actual=actual_digest,
        ok=actual_digest == expected_digest,
    )


def verify_tree_entry(
    path: Path,
    *,
    display_path: str,
    algorithm: HashAlgorithm,
    expected_digest: str,
    expected_files_hashed: int | None,
    chunk_size: int,
    include_suffixes: frozenset[str],
    exclude_patterns: Sequence[str],
    symlinks: SymlinkPolicy,
) -> VerificationResult:
    actual_digest, actual_files_hashed = hash_tree(
        path,
        algorithm,
        chunk_size,
        include_suffixes,
        exclude_patterns,
        symlinks,
    )
    count_matches = (
        expected_files_hashed is None or expected_files_hashed == actual_files_hashed
    )
    digest_matches = actual_digest == expected_digest
    message = ""
    if digest_matches and not count_matches:
        message = f"expected {expected_files_hashed} files got {actual_files_hashed}"
    return VerificationResult(
        mode="tree",
        path=display_path,
        algorithm=algorithm,
        expected=expected_digest,
        actual=actual_digest,
        ok=digest_matches and count_matches,
        message=message,
        expected_files_hashed=expected_files_hashed,
        actual_files_hashed=actual_files_hashed,
    )


def verify_expected_pairs(
    expected_pairs: Sequence[Sequence[str]],
    *,
    algorithm: HashAlgorithm,
    chunk_size: int,
) -> list[VerificationResult]:
    results: list[VerificationResult] = []
    for expected, filename in expected_pairs:
        path = Path(filename)
        if not path.is_file():
            raise ValueError(f"not a file: {path}")
        results.append(
            verify_file_entry(
                path,
                display_path=str(path),
                algorithm=algorithm,
                expected_digest=normalize_digest(expected),
                chunk_size=chunk_size,
            )
        )
    return results


def manifest_items(payload: object) -> list[object]:
    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict):
        for key in ("hashes", "results", "files"):
            items = payload.get(key)
            if isinstance(items, list):
                return items
        if "path" in payload and "digest" in payload:
            return [payload]
    raise ValueError(
        "manifest must be a JSON list or an object containing a hashes/results list"
    )


def manifest_string(item: dict[str, object], key: str, *, index: int) -> str:
    value = item.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(f"manifest entry {index} must contain string field {key!r}")
    return value


def manifest_optional_strings(
    item: dict[str, object],
    key: str,
    *,
    index: int,
) -> tuple[str, ...] | None:
    value = item.get(key)
    if value is None:
        return None
    if not isinstance(value, list) or not all(isinstance(entry, str) for entry in value):
        raise ValueError(f"manifest entry {index} field {key!r} must be a string list")
    return tuple(value)


def load_manifest(path: Path) -> list[HashResult]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"manifest JSON error: {exc}") from exc

    results: list[HashResult] = []
    for index, item in enumerate(manifest_items(payload), start=1):
        if not isinstance(item, dict):
            raise ValueError(f"manifest entry {index} must be an object")

        raw_mode = item.get("mode", "file")
        if raw_mode == "file":
            mode: Mode = "file"
        elif raw_mode == "tree":
            mode = "tree"
        else:
            raise ValueError(f"manifest entry {index} has unsupported mode: {raw_mode}")

        raw_files_hashed = item.get("files_hashed")
        if raw_files_hashed is not None and not isinstance(raw_files_hashed, int):
            raise ValueError(f"manifest entry {index} field 'files_hashed' must be an integer")

        include_suffixes = manifest_optional_strings(item, "include_suffixes", index=index)
        exclude_patterns = manifest_optional_strings(item, "exclude_patterns", index=index)
        symlink_policy: SymlinkPolicy | None = None
        if item.get("symlink_policy") is not None:
            symlink_policy = normalize_symlink_policy(item["symlink_policy"])

        results.append(
            HashResult(
                mode=mode,
                name=str(item.get("name", "")),
                path=manifest_string(item, "path", index=index),
                algorithm=normalize_algorithm(manifest_string(item, "algorithm", index=index)),
                digest=normalize_digest(manifest_string(item, "digest", index=index)),
                files_hashed=raw_files_hashed,
                include_suffixes=include_suffixes,
                exclude_patterns=exclude_patterns,
                symlink_policy=symlink_policy,
            )
        )
    if not results:
        raise ValueError("manifest contains no hash entries")
    return results


def resolve_manifest_entry_path(entry_path: str) -> Path:
    path = Path(entry_path)
    if path.is_absolute():
        return path
    return Path.cwd() / path


def verify_manifest(manifest_path: Path, chunk_size: int) -> list[VerificationResult]:
    entries = load_manifest(manifest_path)
    results: list[VerificationResult] = []
    for entry in entries:
        path = resolve_manifest_entry_path(entry.path)
        if entry.mode == "file":
            if not path.is_file():
                raise ValueError(f"manifest entry is not a file: {entry.path}")
            results.append(
                verify_file_entry(
                    path,
                    display_path=entry.path,
                    algorithm=entry.algorithm,
                    expected_digest=entry.digest,
                    chunk_size=chunk_size,
                )
            )
            continue

        if not path.is_dir():
            raise ValueError(f"manifest entry is not a directory: {entry.path}")
        include_suffixes = normalize_suffixes(entry.include_suffixes or ())
        exclude_patterns = (
            normalize_exclude_patterns(entry.exclude_patterns)
            if entry.exclude_patterns is not None
            else defaulted_exclude_patterns((), use_defaults=True)
        )
        results.append(
            verify_tree_entry(
                path,
                display_path=entry.path,
                algorithm=entry.algorithm,
                expected_digest=entry.digest,
                expected_files_hashed=entry.files_hashed,
                chunk_size=chunk_size,
                include_suffixes=include_suffixes,
                exclude_patterns=exclude_patterns,
                symlinks=entry.symlink_policy or "error",
            )
        )
    return results


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="hash.py",
        usage=(
            "hash.py [options] FILE [FILE ...]\n"
            "       hash.py [options] --tree DIR\n"
            "       hash.py [options] --expected DIGEST FILE\n"
            "       hash.py [options] --check manifest.json"
        ),
        description="Hash files or deterministic directory trees.",
        epilog=dedent(HELP_EPILOG).strip(),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "paths",
        nargs="*",
        type=Path,
        metavar="FILE",
        help="File mode input. Pass one or more files unless using --tree.",
    )

    hash_options = parser.add_argument_group("hash options")
    hash_options.add_argument(
        "-a",
        "--algorithm",
        metavar="ALG",
        type=normalize_algorithm,
        default=DEFAULT_ALGORITHM,
        help=f"Hash algorithm: sha256, sha512, blake2b, or md5. Default: {DEFAULT_ALGORITHM}.",
    )
    hash_options.add_argument(
        "--chunk-size",
        metavar="SIZE",
        type=parse_chunk_size,
        default=DEFAULT_CHUNK_SIZE,
        help="Read chunk size per file. Accepts b, kb, mb, or gb suffixes. Default: 1mb.",
    )

    output_options = parser.add_argument_group("output options")
    output_options.add_argument(
        "--format",
        metavar="FORMAT",
        choices=["plain", "json", "release"],
        default="plain",
        help="Output format: plain, json, or release. Default: plain.",
    )
    output_options.add_argument(
        "--name",
        metavar="NAME",
        help="Display name for one file or tree, mainly useful with --format release.",
    )

    verify_options = parser.add_argument_group("verify mode options")
    verify_options.add_argument(
        "--expected",
        nargs=2,
        action="append",
        metavar=("DIGEST", "FILE"),
        help="Verify one file against an expected digest. Repeatable.",
    )
    verify_options.add_argument(
        "--check",
        dest="check_manifest",
        metavar="MANIFEST",
        type=Path,
        help="Verify every entry in a JSON manifest emitted by --format json.",
    )

    tree_options = parser.add_argument_group("tree mode options")
    tree_options.add_argument(
        "--tree",
        metavar="DIR",
        type=Path,
        help="Hash a directory tree deterministically instead of individual files.",
    )
    tree_options.add_argument(
        "--include",
        action="append",
        default=[],
        metavar="SUFFIX",
        help=(
            "Only include files with this suffix in tree mode. Repeatable, "
            "e.g. --include .py --include .rs."
        ),
    )
    tree_options.add_argument(
        "--exclude",
        action="append",
        default=[],
        metavar="PATTERN",
        help=(
            "Exclude a relative path or glob pattern in tree mode. Bare names match any path "
            "component. Repeatable."
        ),
    )
    tree_options.add_argument(
        "--no-default-excludes",
        action="store_true",
        help=(
            "Disable default tree excludes: "
            + ", ".join(DEFAULT_EXCLUDE_PATTERNS)
            + "."
        ),
    )
    tree_options.add_argument(
        "--symlinks",
        metavar="POLICY",
        choices=["follow", "skip", "error"],
        default="error",
        help="Tree-mode symlink policy: follow, skip, or error. Default: error.",
    )
    tree_options.add_argument(
        "--list-files",
        action="store_true",
        help="Include per-file path, size, algorithm, and digest entries in tree output.",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    try:
        algorithm: HashAlgorithm = args.algorithm
        output_format: OutputFormat = args.format
        symlinks: SymlinkPolicy = args.symlinks
        include_suffixes = normalize_suffixes(args.include)
        exclude_patterns = defaulted_exclude_patterns(
            args.exclude,
            use_defaults=not args.no_default_excludes,
        )
        verify_mode_count = sum(
            [
                args.expected is not None,
                args.check_manifest is not None,
            ]
        )

        if verify_mode_count > 1:
            raise ValueError("--expected and --check cannot be combined")

        if args.expected is not None:
            if args.paths or args.tree is not None:
                raise ValueError("file paths and --tree cannot be combined with --expected")
            if args.include or args.exclude or args.no_default_excludes or args.list_files:
                raise ValueError("tree options cannot be combined with --expected")
            verification = verify_expected_pairs(
                args.expected,
                algorithm=algorithm,
                chunk_size=args.chunk_size,
            )
            print(render_verification(verification, output_format))
            return EXIT_OK if all(result.ok for result in verification) else EXIT_VERIFY_FAILED

        if args.check_manifest is not None:
            if args.paths or args.tree is not None:
                raise ValueError("file paths and --tree cannot be combined with --check")
            if args.include or args.exclude or args.no_default_excludes or args.list_files:
                raise ValueError("tree options cannot be combined with --check")
            verification = verify_manifest(args.check_manifest, args.chunk_size)
            print(render_verification(verification, output_format))
            return EXIT_OK if all(result.ok for result in verification) else EXIT_VERIFY_FAILED

        if args.tree is not None:
            if args.paths:
                raise ValueError("file paths cannot be combined with --tree")
            results = [
                build_tree_result(
                    root=args.tree,
                    algorithm=algorithm,
                    chunk_size=args.chunk_size,
                    include_suffixes=include_suffixes,
                    exclude_patterns=exclude_patterns,
                    symlinks=symlinks,
                    name=args.name,
                    list_files=args.list_files,
                )
            ]
        else:
            if args.list_files:
                raise ValueError("--list-files is only valid with --tree")
            if args.include:
                raise ValueError("--include is only valid with --tree")
            if args.exclude or args.no_default_excludes:
                raise ValueError("--exclude and --no-default-excludes are only valid with --tree")
            if not args.paths:
                raise ValueError("at least one file path is required unless --tree is used")
            results = build_file_results(
                paths=args.paths,
                algorithm=algorithm,
                chunk_size=args.chunk_size,
                name=args.name,
            )

        print(render(results, output_format))
        return EXIT_OK

    except (OSError, RuntimeError, ValueError, argparse.ArgumentTypeError) as exc:
        parser.exit(EXIT_ERROR, f"error: {exc}\n")


if __name__ == "__main__":
    raise SystemExit(main())
