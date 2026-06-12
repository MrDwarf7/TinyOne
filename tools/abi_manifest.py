#!/usr/bin/env python3
"""TinyOne C ABI symbol manifest and header drift checker.

This tool intentionally uses only the Python standard library. The default
check compares exported `extern "C"` Rust symbols in `TinyOne/src/ffi.rs`
against the generated TinyLang C header `tinylang.h`.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FFI = ROOT / "TinyOne" / "src" / "ffi.rs"
DEFAULT_GENERATED_HEADER = ROOT / "tinylang.h"
DEFAULT_HEADER = DEFAULT_GENERATED_HEADER
DEFAULT_CBINDGEN_CONFIG = ROOT / "cbindgen.toml"
DEFAULT_CBINDGEN_SOURCE = DEFAULT_FFI
EXPECTED_PACKAGE_NAME = "tinylang"


RUST_EXPORT_RE = re.compile(
    r"#\s*\[\s*(?:unsafe\s*\(\s*)?no_mangle\s*\)?\s*\]\s*"
    r"(?:\s*///[^\n]*\n|\s*#\[[^\]]*\]\s*)*"
    r"pub\s+(?:unsafe\s+)?extern\s+\"C\"\s+fn\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)
HEADER_SYMBOL_RE = re.compile(r"\b(tinyone_[A-Za-z0-9_]+)\s*\(")


@dataclass(frozen=True)
class AbiSymbols:
    rust_symbols: tuple[str, ...]
    header_symbols: tuple[str, ...]
    rust_path: Path
    header_path: Path
    rust_sha256: str
    header_sha256: str

    @property
    def missing_from_header(self) -> tuple[str, ...]:
        return tuple(sorted(set(self.rust_symbols) - set(self.header_symbols)))

    @property
    def missing_from_rust(self) -> tuple[str, ...]:
        return tuple(sorted(set(self.header_symbols) - set(self.rust_symbols)))

    @property
    def has_drift(self) -> bool:
        return bool(self.missing_from_header or self.missing_from_rust)


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as error:
        raise SystemExit(f"missing input file: {path}") from error


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def strip_c_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return re.sub(r"//.*", "", text)


def rust_exports(path: Path) -> tuple[str, ...]:
    text = read_text(path)
    return tuple(sorted(set(RUST_EXPORT_RE.findall(text))))


def header_symbols(path: Path) -> tuple[str, ...]:
    text = strip_c_comments(read_text(path))
    return tuple(sorted(set(HEADER_SYMBOL_RE.findall(text))))


def collect_symbols(ffi_path: Path, header_path: Path) -> AbiSymbols:
    return AbiSymbols(
        rust_symbols=rust_exports(ffi_path),
        header_symbols=header_symbols(header_path),
        rust_path=ffi_path,
        header_path=header_path,
        rust_sha256=sha256_file(ffi_path),
        header_sha256=sha256_file(header_path),
    )


def rel(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def yaml_list(values: tuple[str, ...]) -> list[str]:
    if not values:
        return ["  []"]
    return [f"  - {value}" for value in values]


def yaml_nested_list(values: tuple[str, ...]) -> list[str]:
    if not values:
        return ["    []"]
    return [f"    - {value}" for value in values]


def render_manifest(symbols: AbiSymbols) -> str:
    lines = [
        "# TinyOne ABI symbol manifest",
        "# Deterministic symbol inventory only; not a stable ABI claim.",
        "format: tinyone-abi-symbol-manifest-v1",
        "inputs:",
        f"  rust_ffi: {rel(symbols.rust_path)}",
        f"  rust_ffi_sha256: {symbols.rust_sha256}",
        f"  c_header: {rel(symbols.header_path)}",
        f"  c_header_sha256: {symbols.header_sha256}",
        "rust_symbols:",
        *yaml_list(symbols.rust_symbols),
        "header_symbols:",
        *yaml_list(symbols.header_symbols),
        "drift:",
        "  missing_from_header:",
        *yaml_nested_list(symbols.missing_from_header),
        "  missing_from_rust_exports:",
        *yaml_nested_list(symbols.missing_from_rust),
    ]
    return "\n".join(lines) + "\n"


def validate_tinylang_crate_dir(crate_dir: Path) -> Path:
    manifest = crate_dir / "Cargo.toml"
    if not manifest.is_file():
        raise ValueError(f"crate directory {rel(crate_dir)} must contain Cargo.toml")

    try:
        data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise ValueError(f"{rel(manifest)} is not valid TOML: {error}") from error

    package = data.get("package")
    name = package.get("name") if isinstance(package, dict) else None
    if name != EXPECTED_PACKAGE_NAME:
        raise ValueError(
            f"{rel(manifest)} must be package name {EXPECTED_PACKAGE_NAME!r}; got {name!r}"
        )

    return crate_dir


def validate_cbindgen_source(source: Path) -> Path:
    if not source.is_file():
        raise ValueError(f"cbindgen source does not exist: {rel(source)}")
    return source


def command_manifest(args: argparse.Namespace) -> int:
    symbols = collect_symbols(args.ffi, args.header)
    print(render_manifest(symbols), end="")
    return 0


def command_check(args: argparse.Namespace) -> int:
    symbols = collect_symbols(args.ffi, args.header)
    if not symbols.has_drift:
        print(
            f"ABI header drift check passed: {len(symbols.rust_symbols)} symbols match "
            f"{rel(symbols.rust_path)} and {rel(symbols.header_path)}."
        )
        return 0

    print("ABI header drift check failed.")
    if symbols.missing_from_header:
        print("Rust exports missing from header:")
        for symbol in symbols.missing_from_header:
            print(f"  - {symbol}")
    if symbols.missing_from_rust:
        print("Header declarations missing from Rust exports:")
        for symbol in symbols.missing_from_rust:
            print(f"  - {symbol}")
    return 1


def command_generate_header(args: argparse.Namespace) -> int:
    try:
        crate_dir = validate_tinylang_crate_dir(args.crate_dir)
        source = validate_cbindgen_source(args.ffi_source)
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 2

    config = args.config
    if config is not None and not config.is_file():
        print(f"cbindgen config does not exist: {rel(config)}", file=sys.stderr)
        return 2

    cbindgen = shutil.which(args.cbindgen)
    if cbindgen is None:
        print(
            "cbindgen is not available on PATH; cannot generate tinylang.h.",
            file=sys.stderr,
        )
        print(
            "Install cbindgen separately or run `Tools/abi_manifest.py check` "
            "for the no-dependency drift check.",
            file=sys.stderr,
        )
        return 2

    output = args.output
    command = [
        cbindgen,
        "--lang",
        "c",
        "--output",
        str(output),
    ]
    if config is not None:
        command.extend(["--config", str(config)])
    command.append(str(source))
    try:
        subprocess.run(command, cwd=ROOT, check=True)
    except subprocess.CalledProcessError as error:
        return error.returncode
    print(f"generated {rel(output)} with cbindgen")
    return 0


def parser() -> argparse.ArgumentParser:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--ffi",
        type=Path,
        default=DEFAULT_FFI,
        help=f"Rust FFI source to inspect (default: {rel(DEFAULT_FFI)})",
    )
    common.add_argument(
        "--header",
        type=Path,
        default=DEFAULT_HEADER,
        help=f"C header to inspect (default: {rel(DEFAULT_HEADER)})",
    )

    root = argparse.ArgumentParser(
        description="Check TinyOne C ABI header drift and emit symbol manifests."
    )
    subcommands = root.add_subparsers(dest="command", required=True)

    manifest = subcommands.add_parser(
        "manifest",
        parents=[common],
        help="print deterministic ABI symbol manifest",
    )
    manifest.set_defaults(func=command_manifest)

    check = subcommands.add_parser(
        "check",
        parents=[common],
        help="fail if Rust exports and header symbols drift",
    )
    check.set_defaults(func=command_check)

    generate = subcommands.add_parser(
        "generate-header",
        help="generate planned tinylang.h with cbindgen when cbindgen is installed",
    )
    generate.add_argument(
        "--crate-dir",
        type=Path,
        default=ROOT / "TinyOne",
        help="Rust crate directory whose Cargo.toml must identify the tinylang package",
    )
    generate.add_argument(
        "--ffi-source",
        type=Path,
        default=DEFAULT_CBINDGEN_SOURCE,
        help=f"Rust FFI source passed to cbindgen (default: {rel(DEFAULT_CBINDGEN_SOURCE)})",
    )
    generate.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_GENERATED_HEADER,
        help=f"generated header path (default: {rel(DEFAULT_GENERATED_HEADER)})",
    )
    generate.add_argument(
        "--cbindgen",
        default="cbindgen",
        help="cbindgen executable name or path",
    )
    generate.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CBINDGEN_CONFIG if DEFAULT_CBINDGEN_CONFIG.exists() else None,
        help=f"cbindgen config path (default: {rel(DEFAULT_CBINDGEN_CONFIG)} when present)",
    )
    generate.set_defaults(func=command_generate_header)

    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
