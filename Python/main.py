#!/usr/bin/env python3
"""
TinyOne: single-file stdlib-only compiler/VM/JIT implementation.

Language:
    import "math.to" as math
    fn double(n) { return n * 2 }
    struct Pair { left, right }
    let x = 1 + 2 * 3
    while x < 10 { x = x + 1 }
    let pair = Pair("left", [1, 2, 3])
    let mem = buffer(16)
    print unsafe ptr_load(fieldptr(pair, "left"))
    print unsafe write8(ptr(mem, 0), 255)
    print x

Design constraints:
    - Python stdlib only
    - Single file
    - Maintainable Python implementation

Runtime model:
    Source -> tokens -> bytecode -> [peephole] -> [verify] -> VM or JIT.

Memory model:
    TinyMemory is a zero-initialized stack frame addressed by Slot handles.
    Heap-backed strings, arrays, structs, buffers, pointer cells, and raw
    pointer values live in TinyHeap/TinyRuntimeContext across function calls.

JIT model:
    Branch-free main programs keep the locals-based path: each virtual stack
    slot maps to a named Python local (_s0, _s1, ...). Programs with loops or
    functions use generated Python dispatch code so absolute branch targets and
    calls remain bytecode-compatible with the VM.

Optimization:
    PeepholeOptimizer folds PUSH_INT + PUSH_INT + <binop/cmp> into a single
    PUSH_INT in branch-free chunks, running to convergence. Folding happens
    before verification and before JIT codegen.

Verification:
    BytecodeVerifier performs a static control-flow-aware stack-depth check
    before any execution. Stack imbalance is a compile-time error, not a
    runtime error.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
from pathlib import Path
import sys
from dataclasses import dataclass
from enum import IntEnum
from types import FunctionType
from typing import Callable, Final, Iterable, NewType, TextIO

Slot = NewType("Slot", int)

LOGGER = logging.getLogger("tinyone")


class TinyOneError(Exception):
    """Base TinyOne user-facing error."""


class CompileError(TinyOneError):
    """Raised for lexing/parsing/compilation/verification failures."""


class RuntimeTinyOneError(TinyOneError):
    """Raised for runtime failures."""


class SourceMap:
    """Maps byte offsets to user-facing file/line/column diagnostics."""

    __slots__ = ("filename", "source", "_line_starts")

    def __init__(self, source: str, filename: str = "<source>") -> None:
        self.filename = filename
        self.source = source
        self._line_starts = self._build_line_starts(source)

    @staticmethod
    def _build_line_starts(source: str) -> tuple[int, ...]:
        starts = [0]
        for index, char in enumerate(source):
            if char == "\n":
                starts.append(index + 1)
        return tuple(starts)

    def line_col(self, pos: int) -> tuple[int, int]:
        pos = max(0, min(pos, len(self.source)))
        starts = self._line_starts
        low = 0
        high = len(starts)
        while low + 1 < high:
            mid = (low + high) // 2
            if starts[mid] <= pos:
                low = mid
            else:
                high = mid
        return low + 1, pos - starts[low] + 1

    def format(self, message: str, pos: int, end: int | None = None) -> str:
        line, column = self.line_col(pos)
        line_start = self._line_starts[line - 1]
        next_line_start = (
            self._line_starts[line] if line < len(self._line_starts) else len(self.source)
        )
        line_text = self.source[line_start:next_line_start].rstrip("\n")
        span_end = pos + 1 if end is None else max(pos + 1, end)
        width = max(1, min(span_end, next_line_start) - pos)
        caret = " " * (column - 1) + "^" * width
        return f"{self.filename}:{line}:{column}: {message}\n{line_text}\n{caret}"


class TokenKind(IntEnum):
    INT = 1
    IDENT = 2
    STRING = 3
    LET = 4
    PRINT = 5
    FN = 6
    RETURN = 7
    WHILE = 8
    IF = 9
    ELSE = 10
    BREAK = 11
    CONTINUE = 12
    STRUCT = 13
    IMPORT = 14
    EXPORT = 15
    AS = 16
    SET = 17
    UNSAFE = 18
    PLUS = 19
    MINUS = 20
    STAR = 21
    SLASH = 22
    EQUAL = 23
    EQEQ = 24
    BANG_EQUAL = 25
    LT = 26
    LTE = 27
    GT = 28
    GTE = 29
    LPAREN = 30
    RPAREN = 31
    LBRACE = 32
    RBRACE = 33
    LBRACKET = 34
    RBRACKET = 35
    DOT = 36
    COMMA = 37
    EOF = 38
    NULL = 39


@dataclass(frozen=True, slots=True)
class Token:
    kind: TokenKind
    text: str
    pos: int
    end: int


_KEYWORDS: Final[dict[str, TokenKind]] = {
    "let": TokenKind.LET,
    "print": TokenKind.PRINT,
    "fn": TokenKind.FN,
    "return": TokenKind.RETURN,
    "while": TokenKind.WHILE,
    "if": TokenKind.IF,
    "else": TokenKind.ELSE,
    "break": TokenKind.BREAK,
    "continue": TokenKind.CONTINUE,
    "struct": TokenKind.STRUCT,
    "import": TokenKind.IMPORT,
    "export": TokenKind.EXPORT,
    "as": TokenKind.AS,
    "set": TokenKind.SET,
    "unsafe": TokenKind.UNSAFE,
    "null": TokenKind.NULL,
}

_TWO_CHAR_TOKENS: Final[dict[str, TokenKind]] = {
    "==": TokenKind.EQEQ,
    "!=": TokenKind.BANG_EQUAL,
    "<=": TokenKind.LTE,
    ">=": TokenKind.GTE,
}

_SINGLE_CHAR_TOKENS: Final[dict[str, TokenKind]] = {
    "+": TokenKind.PLUS,
    "-": TokenKind.MINUS,
    "*": TokenKind.STAR,
    "/": TokenKind.SLASH,
    "=": TokenKind.EQUAL,
    "<": TokenKind.LT,
    ">": TokenKind.GT,
    "(": TokenKind.LPAREN,
    ")": TokenKind.RPAREN,
    "{": TokenKind.LBRACE,
    "}": TokenKind.RBRACE,
    "[": TokenKind.LBRACKET,
    "]": TokenKind.RBRACKET,
    ".": TokenKind.DOT,
    ",": TokenKind.COMMA,
}


class Lexer:
    """Hand-written lexer optimized for one pass over the source string."""

    __slots__ = ("_source", "_length", "_pos", "_source_map")

    def __init__(self, source: str, filename: str = "<source>") -> None:
        self._source = source
        self._length = len(source)
        self._pos = 0
        self._source_map = SourceMap(source, filename)

    def tokenize(self) -> list[Token]:
        source = self._source
        length = self._length
        pos = self._pos
        tokens: list[Token] = []
        append = tokens.append

        while pos < length:
            ch = source[pos]

            if ch.isspace():
                pos += 1
                continue

            if ch == "#":
                pos += 1
                while pos < length and source[pos] != "\n":
                    pos += 1
                continue

            if "0" <= ch <= "9":
                start = pos
                pos += 1
                while pos < length and "0" <= source[pos] <= "9":
                    pos += 1
                append(Token(TokenKind.INT, source[start:pos], start, pos))
                continue

            if ch == '"':
                start = pos
                pos += 1
                chars: list[str] = []
                while pos < length and source[pos] != '"':
                    if source[pos] == "\n":
                        raise self._error("Unterminated string literal", start, pos)
                    if source[pos] == "\\":
                        pos += 1
                        if pos >= length:
                            raise self._error("Unterminated string escape", start, pos)
                        escaped = source[pos]
                        if escaped == "n":
                            chars.append("\n")
                        elif escaped == "t":
                            chars.append("\t")
                        elif escaped in ('"', "\\"):
                            chars.append(escaped)
                        else:
                            raise self._error(f"Unknown string escape \\{escaped}", pos, pos + 1)
                    else:
                        chars.append(source[pos])
                    pos += 1
                if pos >= length:
                    raise self._error("Unterminated string literal", start, pos)
                pos += 1
                append(Token(TokenKind.STRING, "".join(chars), start, pos))
                continue

            if ch == "_" or ch.isalpha():
                start = pos
                pos += 1
                while pos < length:
                    c = source[pos]
                    if not (c == "_" or c.isalpha() or ("0" <= c <= "9")):
                        break
                    pos += 1
                text = source[start:pos]
                append(Token(_KEYWORDS.get(text, TokenKind.IDENT), text, start, pos))
                continue

            if pos + 1 < length:
                pair = source[pos : pos + 2]
                kind = _TWO_CHAR_TOKENS.get(pair)
                if kind is not None:
                    append(Token(kind, pair, pos, pos + 2))
                    pos += 2
                    continue

            kind = _SINGLE_CHAR_TOKENS.get(ch)
            if kind is None:
                raise self._error(f"Unexpected character {ch!r}", pos, pos + 1)
            append(Token(kind, ch, pos, pos + 1))
            pos += 1

        append(Token(TokenKind.EOF, "", pos, pos))
        self._pos = pos
        return tokens

    def _error(self, message: str, pos: int, end: int | None = None) -> CompileError:
        return CompileError(self._source_map.format(message, pos, end))


class Op(IntEnum):
    PUSH_INT = 1
    LOAD = 2
    STORE = 3
    ADD = 4
    SUB = 5
    MUL = 6
    DIV = 7
    NEG = 8
    PRINT = 9
    LT = 10
    LTE = 11
    GT = 12
    GTE = 13
    EQ = 14
    NE = 15
    JUMP = 16
    JUMP_IF_ZERO = 17
    CALL = 18
    RETURN = 19
    HALT = 20
    PUSH_STRING = 21
    MAKE_ARRAY = 22
    INDEX = 23
    SET_INDEX = 24
    MAKE_STRUCT = 25
    GET_FIELD = 26
    SET_FIELD = 27
    BUILTIN = 28
    PUSH_NULL = 29


_COMPARISON_OPS: Final[dict[TokenKind, Op]] = {
    TokenKind.LT: Op.LT,
    TokenKind.LTE: Op.LTE,
    TokenKind.GT: Op.GT,
    TokenKind.GTE: Op.GTE,
    TokenKind.EQEQ: Op.EQ,
    TokenKind.BANG_EQUAL: Op.NE,
}

_ADDITIVE_OPS: Final[dict[TokenKind, Op]] = {
    TokenKind.PLUS: Op.ADD,
    TokenKind.MINUS: Op.SUB,
}

_TERM_OPS: Final[dict[TokenKind, Op]] = {
    TokenKind.STAR: Op.MUL,
    TokenKind.SLASH: Op.DIV,
}


@dataclass(frozen=True, slots=True)
class Instr:
    op: Op
    arg: int = 0
    arg2: int = 0


@dataclass(frozen=True, slots=True)
class Function:
    name: str
    param_count: int
    code: tuple[Instr, ...]
    slot_count: int
    names: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class StructDef:
    name: str
    fields: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class ModuleImportDef:
    alias: str
    path: str
    module: str
    resolved: str


@dataclass(frozen=True, slots=True)
class ModuleDef:
    name: str
    path: str
    imports: tuple[ModuleImportDef, ...] = ()
    exported_functions: tuple[str, ...] = ()
    exported_structs: tuple[str, ...] = ()


@dataclass(slots=True)
class ModuleInfo:
    name: str
    path: str
    function_exports: dict[str, int]
    struct_exports: dict[str, int]
    all_functions: set[str]
    all_structs: set[str]
    imports: list[ModuleImportDef]
    finalized: bool = False


@dataclass(frozen=True, slots=True)
class Program:
    code: tuple[Instr, ...]
    slot_count: int
    names: tuple[str, ...]
    functions: tuple[Function, ...] = ()
    strings: tuple[str, ...] = ()
    structs: tuple[StructDef, ...] = ()
    fields: tuple[str, ...] = ()
    modules: tuple[ModuleDef, ...] = ()

    @property
    def fingerprint(self) -> str:
        hasher = hashlib.blake2b(digest_size=16)
        self._hash_code(hasher, self.code)
        hasher.update(self.slot_count.to_bytes(8, "little", signed=False))
        for name in self.names:
            encoded = name.encode("utf-8")
            hasher.update(len(encoded).to_bytes(4, "little", signed=False))
            hasher.update(encoded)
        hasher.update(len(self.functions).to_bytes(8, "little", signed=False))
        for function in self.functions:
            encoded_name = function.name.encode("utf-8")
            hasher.update(len(encoded_name).to_bytes(4, "little", signed=False))
            hasher.update(encoded_name)
            hasher.update(function.param_count.to_bytes(8, "little", signed=False))
            hasher.update(function.slot_count.to_bytes(8, "little", signed=False))
            self._hash_code(hasher, function.code)
        for text in self.strings:
            encoded = text.encode("utf-8")
            hasher.update(len(encoded).to_bytes(8, "little", signed=False))
            hasher.update(encoded)
        for struct in self.structs:
            encoded_name = struct.name.encode("utf-8")
            hasher.update(len(encoded_name).to_bytes(4, "little", signed=False))
            hasher.update(encoded_name)
            hasher.update(len(struct.fields).to_bytes(4, "little", signed=False))
            for field in struct.fields:
                encoded_field = field.encode("utf-8")
                hasher.update(len(encoded_field).to_bytes(4, "little", signed=False))
                hasher.update(encoded_field)
        for field in self.fields:
            encoded = field.encode("utf-8")
            hasher.update(len(encoded).to_bytes(4, "little", signed=False))
            hasher.update(encoded)
        for module in self.modules:
            encoded_name = module.name.encode("utf-8")
            encoded_path = module.path.encode("utf-8")
            hasher.update(len(encoded_name).to_bytes(4, "little", signed=False))
            hasher.update(encoded_name)
            hasher.update(len(encoded_path).to_bytes(4, "little", signed=False))
            hasher.update(encoded_path)
            for imports in (
                tuple(item.alias for item in module.imports),
                tuple(item.path for item in module.imports),
                tuple(item.module for item in module.imports),
                tuple(item.resolved for item in module.imports),
                module.exported_functions,
                module.exported_structs,
            ):
                hasher.update(len(imports).to_bytes(4, "little", signed=False))
                for item in imports:
                    encoded_item = item.encode("utf-8")
                    hasher.update(len(encoded_item).to_bytes(4, "little", signed=False))
                    hasher.update(encoded_item)
        return hasher.hexdigest()

    @staticmethod
    def _hash_code(hasher: object, code: tuple[Instr, ...]) -> None:
        for instr in code:
            hasher.update(int(instr.op).to_bytes(2, "little", signed=False))
            hasher.update(int(instr.arg).to_bytes(16, "little", signed=True))
            hasher.update(int(instr.arg2).to_bytes(16, "little", signed=True))

    def to_artifact(self) -> dict[str, object]:
        return {
            "format": "tinyone-bytecode",
            "version": 1,
            "code": _encode_code(self.code),
            "slot_count": self.slot_count,
            "names": list(self.names),
            "functions": [
                {
                    "name": function.name,
                    "param_count": function.param_count,
                    "code": _encode_code(function.code),
                    "slot_count": function.slot_count,
                    "names": list(function.names),
                }
                for function in self.functions
            ],
            "strings": list(self.strings),
            "structs": [
                {"name": struct.name, "fields": list(struct.fields)} for struct in self.structs
            ],
            "fields": list(self.fields),
            "modules": [
                {
                    "name": module.name,
                    "path": module.path,
                    "imports": [
                        {
                            "alias": item.alias,
                            "path": item.path,
                            "module": item.module,
                            "resolved": item.resolved,
                        }
                        for item in module.imports
                    ],
                    "exported_functions": list(module.exported_functions),
                    "exported_structs": list(module.exported_structs),
                }
                for module in self.modules
            ],
        }

    @staticmethod
    def from_artifact(data: object) -> "Program":
        if not isinstance(data, dict):
            raise CompileError("Artifact must be a JSON object")
        if data.get("format") != "tinyone-bytecode" or data.get("version") != 1:
            raise CompileError("Unsupported TinyOne artifact format")
        functions = tuple(
            Function(
                str(item["name"]),
                int(item["param_count"]),
                _decode_code(item["code"]),
                int(item["slot_count"]),
                tuple(str(name) for name in item["names"]),
            )
            for item in _expect_list(data.get("functions"), "functions")
        )
        program = Program(
            _decode_code(data.get("code")),
            int(data.get("slot_count", 0)),
            tuple(str(name) for name in _expect_list(data.get("names"), "names")),
            functions,
            tuple(str(text) for text in _expect_list(data.get("strings"), "strings")),
            tuple(
                StructDef(
                    str(item["name"]),
                    tuple(str(field) for field in _expect_list(item["fields"], "struct fields")),
                )
                for item in _expect_list(data.get("structs"), "structs")
            ),
            tuple(str(field) for field in _expect_list(data.get("fields"), "fields")),
            tuple(
                ModuleDef(
                    str(item["name"]),
                    str(item["path"]),
                    tuple(
                        ModuleImportDef(
                            str(import_item["alias"]),
                            str(import_item["path"]),
                            str(import_item["module"]),
                            str(import_item["resolved"]),
                        )
                        for import_item in _expect_list(item.get("imports"), "module imports")
                    ),
                    tuple(
                        str(name)
                        for name in _expect_list(
                            item.get("exported_functions"), "module function exports"
                        )
                    ),
                    tuple(
                        str(name)
                        for name in _expect_list(
                            item.get("exported_structs"), "module struct exports"
                        )
                    ),
                )
                for item in _optional_list(data.get("modules"), "modules")
            ),
        )
        BytecodeVerifier.verify(program)
        return program


def _encode_code(code: tuple[Instr, ...]) -> list[dict[str, int | str]]:
    return [
        {"op": instr.op.name, "arg": instr.arg, "arg2": instr.arg2}
        for instr in code
    ]


def _decode_code(data: object) -> tuple[Instr, ...]:
    return tuple(
        Instr(Op[str(item["op"])], int(item.get("arg", 0)), int(item.get("arg2", 0)))
        for item in _expect_list(data, "code")
    )


def _expect_list(value: object, name: str) -> list[object]:
    if not isinstance(value, list):
        raise CompileError(f"Artifact field {name!r} must be a list")
    return value


def _optional_list(value: object, name: str) -> list[object]:
    if value is None:
        return []
    return _expect_list(value, name)


@dataclass(slots=True)
class CompilerSharedState:
    function_indexes: dict[str, int]
    functions: list[Function]
    struct_indexes: dict[str, int]
    structs: list[StructDef]
    field_indexes: dict[str, int]
    fields: list[str]
    string_indexes: dict[str, int]
    strings: list[str]
    modules: dict[str, ModuleInfo]
    loading_modules: set[str]
    module_defs: list[ModuleDef]
    module_name_owners: dict[str, str]

    @staticmethod
    def fresh() -> "CompilerSharedState":
        return CompilerSharedState({}, [], {}, [], {}, [], {}, [], {}, set(), [], {})


@dataclass(frozen=True, slots=True)
class BuiltinDef:
    name: str
    min_args: int
    max_args: int
    requires_unsafe: bool = False


_BUILTINS: Final[tuple[BuiltinDef, ...]] = (
    BuiltinDef("len", 1, 1),
    BuiltinDef("array", 2, 2),
    BuiltinDef("alloc", 1, 1),
    BuiltinDef("load", 1, 1),
    BuiltinDef("store", 2, 2),
    BuiltinDef("free", 1, 1, True),
    BuiltinDef("read", 0, 0),
    BuiltinDef("read_int", 0, 0),
    BuiltinDef("read_str", 0, 0),
    BuiltinDef("to_int", 1, 1),
    BuiltinDef("ptr", 1, 2),
    BuiltinDef("fieldptr", 2, 2),
    BuiltinDef("ptr_addr", 1, 1),
    BuiltinDef("ptr_at", 1, 1, True),
    BuiltinDef("ptr_add", 2, 2, True),
    BuiltinDef("ptr_load", 1, 1, True),
    BuiltinDef("ptr_store", 2, 2, True),
    BuiltinDef("ptr_type", 1, 1),
    BuiltinDef("buffer", 1, 1),
    BuiltinDef("is_null", 1, 1),
    BuiltinDef("ptr_eq", 2, 2),
    BuiltinDef("ptr_ne", 2, 2),
    BuiltinDef("ptr_base", 1, 1),
    BuiltinDef("ptr_offset", 1, 1),
    BuiltinDef("ptr_kind", 1, 1),
    BuiltinDef("ptr_field", 1, 1),
    BuiltinDef("read8", 1, 1, True),
    BuiltinDef("write8", 2, 2, True),
    BuiltinDef("read16", 1, 1, True),
    BuiltinDef("write16", 2, 2, True),
    BuiltinDef("read32", 1, 1, True),
    BuiltinDef("write32", 2, 2, True),
    BuiltinDef("cast_ptr", 2, 2),
    # Phase 1 (Rust-canonical): push/pop ship in the first 35 slots and must
    # mirror Rust order. Without these the Python verifier cannot accept any
    # artifact emitted by Rust that uses push/pop.
    BuiltinDef("push", 2, 2),
    BuiltinDef("pop", 1, 1),
    # ---- Phase 2 stdlib bridge builtins. Order must mirror Rust. ----
    BuiltinDef("vec_new", 0, 0),
    BuiltinDef("vec_clear", 1, 1),
    BuiltinDef("map_new", 0, 0),
    BuiltinDef("map_set", 3, 3),
    BuiltinDef("map_get", 2, 2),
    BuiltinDef("map_has", 2, 2),
    BuiltinDef("map_del", 2, 2),
    BuiltinDef("map_len", 1, 1),
    BuiltinDef("map_keys", 1, 1),
    BuiltinDef("map_values", 1, 1),
    BuiltinDef("io_stdout", 0, 0),
    BuiltinDef("io_stderr", 0, 0),
    BuiltinDef("io_stdin", 0, 0),
    BuiltinDef("io_write", 2, 2),
    BuiltinDef("io_writeln", 2, 2),
    BuiltinDef("io_read_line", 0, 0),
    BuiltinDef("io_flush", 1, 1),
    BuiltinDef("io_capture_stdout", 0, 0),
    BuiltinDef("io_capture_stderr", 0, 0),
    BuiltinDef("str_byte_len", 1, 1),
    BuiltinDef("str_char_len", 1, 1),
    BuiltinDef("str_byte_at", 2, 2),
    BuiltinDef("str_char_at", 2, 2),
    BuiltinDef("str_slice", 3, 3),
    BuiltinDef("str_concat", 2, 2),
    BuiltinDef("str_is_utf8", 1, 1),
    BuiltinDef("str_from_buffer", 1, 1),
    BuiltinDef("mutex_new", 0, 0),
    BuiltinDef("mutex_lock", 1, 1),
    BuiltinDef("mutex_unlock", 1, 1),
    BuiltinDef("atomic_new", 1, 1),
    BuiltinDef("atomic_load", 1, 1),
    BuiltinDef("atomic_store", 2, 2),
    BuiltinDef("atomic_add", 2, 2),
    BuiltinDef("result_ok", 1, 1),
    BuiltinDef("result_err", 1, 1),
    BuiltinDef("result_is_ok", 1, 1),
    BuiltinDef("result_is_err", 1, 1),
    BuiltinDef("result_unwrap", 1, 1),
    BuiltinDef("result_unwrap_err", 1, 1),
    BuiltinDef("option_some", 1, 1),
    BuiltinDef("option_none", 0, 0),
    BuiltinDef("option_is_some", 1, 1),
    BuiltinDef("option_is_none", 1, 1),
    BuiltinDef("option_unwrap", 1, 1),
    BuiltinDef("sys_argc", 0, 0),
    BuiltinDef("sys_argv", 1, 1),
    BuiltinDef("sys_env_has", 1, 1),
    BuiltinDef("sys_env_get", 1, 1),
    BuiltinDef("path_join", 2, 2),
    BuiltinDef("path_basename", 1, 1),
    BuiltinDef("path_dirname", 1, 1),
    BuiltinDef("fs_read", 1, 1, True),
    BuiltinDef("fs_write", 2, 2, True),
    BuiltinDef("fs_exists", 1, 1),
    BuiltinDef("fs_list_dir", 1, 1, True),
    BuiltinDef("math_const", 1, 1),
    BuiltinDef("math_abs", 1, 1),
    BuiltinDef("math_min", 2, 2),
    BuiltinDef("math_max", 2, 2),
    BuiltinDef("logic_and", 2, 2),
    BuiltinDef("logic_or", 2, 2),
    BuiltinDef("logic_not", 1, 1),
    BuiltinDef("logic_xor", 2, 2),
    BuiltinDef("type_of", 1, 1),
    BuiltinDef("type_id", 1, 1),
    BuiltinDef("smallest_fit", 1, 1),
    BuiltinDef("promote", 2, 2),
    BuiltinDef("check_int_range", 2, 2),
    BuiltinDef("typed_add", 3, 3),
    BuiltinDef("typed_sub", 3, 3),
    BuiltinDef("typed_mul", 3, 3),
    BuiltinDef("typed_div", 3, 3),
    BuiltinDef("typed_neg", 2, 2),
    BuiltinDef("assert", 1, 2),
)
_BUILTIN_INDEXES: Final[dict[str, int]] = {
    builtin.name: index for index, builtin in enumerate(_BUILTINS)
}
# Test surface: re-exported so parity tests can compare against the Rust
# canonical name table without touching private internals.
BUILTINS_PUBLIC: Final[tuple[BuiltinDef, ...]] = _BUILTINS


class SymbolTable:
    """Compile-time lexical scopes. Runtime still uses compact slot indexes."""

    __slots__ = ("_scopes", "_names")

    def __init__(self) -> None:
        self._scopes: list[dict[str, Slot]] = [{}]
        self._names: list[str] = []

    def enter_scope(self) -> None:
        self._scopes.append({})

    def exit_scope(self) -> None:
        if len(self._scopes) == 1:
            raise RuntimeError("cannot exit root symbol scope")
        self._scopes.pop()

    def define_current(self, name: str) -> Slot | None:
        if name in self._scopes[-1]:
            return None
        slot = Slot(len(self._names))
        self._scopes[-1][name] = slot
        self._names.append(name)
        return slot

    def get(self, name: str, pos: int) -> Slot:
        for scope in reversed(self._scopes):
            slot = scope.get(name)
            if slot is not None:
                return slot
        raise CompileError(f"Undefined variable {name!r}")

    def contains(self, name: str) -> bool:
        return any(name in scope for scope in self._scopes)

    @property
    def slot_count(self) -> int:
        return len(self._names)

    @property
    def names(self) -> tuple[str, ...]:
        return tuple(self._names)


@dataclass(slots=True)
class LoopContext:
    start: int
    breaks: list[int]


class Compiler:
    """Recursive-descent parser that emits stack-machine bytecode."""

    __slots__ = (
        "_tokens",
        "_index",
        "_current",
        "_source_map",
        "_filename",
        "_resolver",
        "_imported",
        "_module_mode",
        "_module_name",
        "_module_info",
        "_module_imports",
        "_namespaces",
        "_accept_imports",
        "_symbols",
        "_code",
        "_shared",
        "_function_indexes",
        "_functions",
        "_local_function_indexes",
        "_struct_indexes",
        "_structs",
        "_local_struct_indexes",
        "_field_indexes",
        "_fields",
        "_string_indexes",
        "_strings",
        "_in_function",
        "_loops",
        "_unsafe_depth",
    )

    def __init__(
        self,
        source: str,
        *,
        filename: str = "<source>",
        resolver: Callable[[str, str], tuple[str, str]] | None = None,
        imported: set[str] | None = None,
        module_mode: bool = False,
        module_name: str = "",
        shared: CompilerSharedState | None = None,
    ) -> None:
        self._source_map = SourceMap(source, filename)
        self._filename = filename
        self._resolver = resolver
        self._imported = set() if imported is None else imported
        self._module_mode = module_mode
        self._module_name = module_name
        self._accept_imports = True
        self._tokens = Lexer(source, filename).tokenize()
        self._index = 0
        self._current = self._tokens[0]
        self._symbols = SymbolTable()
        self._code: list[Instr] = []
        self._shared = CompilerSharedState.fresh() if shared is None else shared
        self._function_indexes = self._shared.function_indexes
        self._functions = self._shared.functions
        self._local_function_indexes: dict[str, int] = {}
        self._struct_indexes = self._shared.struct_indexes
        self._structs = self._shared.structs
        self._local_struct_indexes: dict[str, int] = {}
        self._field_indexes = self._shared.field_indexes
        self._fields = self._shared.fields
        self._string_indexes = self._shared.string_indexes
        self._strings = self._shared.strings
        self._in_function = False
        self._loops: list[LoopContext] = []
        self._unsafe_depth = 0
        self._module_imports: list[ModuleImportDef] = []
        self._namespaces: dict[str, ModuleInfo] = {}
        self._module_info = None
        if self._module_mode:
            self._module_info = self._shared.modules.get(filename)
            if self._module_info is None:
                module_name_value = _unique_module_name(
                    self._shared,
                    self._module_name or _module_name_from_filename(filename),
                    filename,
                )
                self._module_info = ModuleInfo(
                    module_name_value,
                    filename,
                    {},
                    {},
                    set(),
                    set(),
                    [],
                )
                self._shared.modules[filename] = self._module_info

    def compile(self) -> Program:
        while self._current.kind != TokenKind.EOF:
            if self._current.kind == TokenKind.IMPORT:
                self._import_statement()
            elif self._current.kind == TokenKind.EXPORT:
                self._accept_imports = False
                self._export_declaration()
            elif self._current.kind == TokenKind.STRUCT:
                self._accept_imports = False
                self._struct_definition(exported=False)
            elif self._current.kind == TokenKind.FN:
                self._accept_imports = False
                self._function_definition(exported=False)
            else:
                if self._module_mode:
                    raise self._error(
                        "Imported modules may only contain import, struct, and fn declarations",
                        self._current,
                    )
                self._accept_imports = False
                self._statement()
        self._emit(Op.HALT)
        self._finalize_module()
        return Program(
            tuple(self._code),
            self._symbols.slot_count,
            self._symbols.names,
            tuple(self._functions),
            tuple(self._strings),
            tuple(self._structs),
            tuple(self._fields),
            tuple(self._shared.module_defs),
        )

    def _export_declaration(self) -> None:
        export_token = self._current
        self._eat(TokenKind.EXPORT)
        if self._current.kind == TokenKind.STRUCT:
            self._struct_definition(exported=True)
            return
        if self._current.kind == TokenKind.FN:
            self._function_definition(exported=True)
            return
        raise self._error("Expected function or struct declaration after export", export_token)

    def _finalize_module(self) -> None:
        info = self._module_info
        if info is None or info.finalized:
            return
        info.imports = list(self._module_imports)
        self._shared.module_defs.append(
            ModuleDef(
                info.name,
                info.name,
                tuple(info.imports),
                tuple(sorted(info.function_exports)),
                tuple(sorted(info.struct_exports)),
            )
        )
        info.finalized = True

    def _statement(self) -> None:
        kind = self._current.kind
        if kind == TokenKind.LET:
            self._let_statement()
            return
        if kind == TokenKind.PRINT:
            self._print_statement()
            return
        if kind == TokenKind.WHILE:
            self._while_statement()
            return
        if kind == TokenKind.IF:
            self._if_statement()
            return
        if kind == TokenKind.BREAK:
            self._break_statement()
            return
        if kind == TokenKind.CONTINUE:
            self._continue_statement()
            return
        if kind == TokenKind.IDENT and self._peek_kind(1) == TokenKind.EQUAL:
            self._assignment_statement()
            return
        if kind == TokenKind.RETURN:
            self._return_statement()
            return
        if kind == TokenKind.SET:
            self._set_statement()
            return
        if kind == TokenKind.FN:
            raise self._error(
                "Function definitions are only allowed at top level "
                "and before executable statements",
                self._current,
            )
        if kind in (TokenKind.IMPORT, TokenKind.STRUCT, TokenKind.EXPORT):
            raise self._error(
                "Imports, exports, and struct definitions are only allowed at top level before statements",
                self._current,
            )
        raise self._error("Expected statement", self._current)

    def _let_statement(self) -> None:
        self._eat(TokenKind.LET)
        name = self._current.text
        name_pos = self._current.pos
        self._eat(TokenKind.IDENT)
        if name in self._namespaces:
            raise self._error_at(f"Variable {name!r} conflicts with an imported namespace", name_pos)
        self._eat(TokenKind.EQUAL)
        self._expression()
        slot = self._symbols.define_current(name)
        if slot is None:
            raise self._error_at(f"Variable {name!r} is already defined in this scope", name_pos)
        LOGGER.debug("compiled let", extra={"name": name, "slot": int(slot), "pos": name_pos})
        self._emit(Op.STORE, int(slot))

    def _assignment_statement(self) -> None:
        name_token = self._current
        name = name_token.text
        self._eat(TokenKind.IDENT)
        if name in self._namespaces:
            raise self._error(f"Cannot assign to import namespace {name!r}", name_token)
        slot = self._get_slot(name_token)
        self._eat(TokenKind.EQUAL)
        self._expression()
        self._emit(Op.STORE, int(slot))

    def _print_statement(self) -> None:
        self._eat(TokenKind.PRINT)
        self._expression()
        self._emit(Op.PRINT)

    def _while_statement(self) -> None:
        self._eat(TokenKind.WHILE)
        loop_start = len(self._code)
        self._expression()
        exit_jump = self._emit_placeholder(Op.JUMP_IF_ZERO)
        self._loops.append(LoopContext(loop_start, []))
        self._block()
        loop_context = self._loops.pop()
        self._emit(Op.JUMP, loop_start)
        loop_end = len(self._code)
        self._patch(exit_jump, loop_end)
        for break_jump in loop_context.breaks:
            self._patch(break_jump, loop_end)

    def _if_statement(self) -> None:
        self._eat(TokenKind.IF)
        self._expression()
        false_jump = self._emit_placeholder(Op.JUMP_IF_ZERO)
        self._block()
        if self._current.kind == TokenKind.ELSE:
            end_jump = self._emit_placeholder(Op.JUMP)
            self._patch(false_jump, len(self._code))
            self._eat(TokenKind.ELSE)
            self._block()
            self._patch(end_jump, len(self._code))
        else:
            self._patch(false_jump, len(self._code))

    def _break_statement(self) -> None:
        token = self._current
        self._eat(TokenKind.BREAK)
        if not self._loops:
            raise self._error("Break outside loop", token)
        self._loops[-1].breaks.append(self._emit_placeholder(Op.JUMP))

    def _continue_statement(self) -> None:
        token = self._current
        self._eat(TokenKind.CONTINUE)
        if not self._loops:
            raise self._error("Continue outside loop", token)
        self._emit(Op.JUMP, self._loops[-1].start)

    def _return_statement(self) -> None:
        if not self._in_function:
            raise self._error("Return outside function", self._current)
        self._eat(TokenKind.RETURN)
        self._expression()
        self._emit(Op.RETURN)

    def _set_statement(self) -> None:
        self._eat(TokenKind.SET)
        name_token = self._current
        self._eat(TokenKind.IDENT)
        slot = self._get_slot(name_token)
        self._emit(Op.LOAD, int(slot))

        if self._current.kind == TokenKind.LBRACKET:
            self._eat(TokenKind.LBRACKET)
            self._expression()
            self._eat(TokenKind.RBRACKET)
            self._eat(TokenKind.EQUAL)
            self._expression()
            self._emit(Op.SET_INDEX)
            return

        if self._current.kind == TokenKind.DOT:
            self._eat(TokenKind.DOT)
            field = self._current.text
            self._eat(TokenKind.IDENT)
            field_index = self._intern_field(field)
            self._eat(TokenKind.EQUAL)
            self._expression()
            self._emit(Op.SET_FIELD, field_index)
            return

        raise self._error("Expected indexed or field assignment target after set", self._current)

    def _import_statement(self) -> None:
        token = self._current
        if not self._accept_imports:
            raise self._error("Imports must appear before declarations and statements", token)
        self._eat(TokenKind.IMPORT)
        path_token = self._current
        self._eat(TokenKind.STRING)
        if self._current.kind == TokenKind.AS:
            self._eat(TokenKind.AS)
            alias_token = self._current
            alias = alias_token.text
            self._eat(TokenKind.IDENT)
        else:
            alias_token = path_token
            alias = _default_import_alias(path_token.text)
        if self._resolver is None:
            raise self._error("Imports require compiling from a source file", path_token)
        module_filename, module_source = self._resolver(self._filename, path_token.text)
        if alias in self._namespaces or self._symbols.contains(alias):
            raise self._error(f"Import namespace {alias!r} is already defined", alias_token)
        if alias in _BUILTIN_INDEXES:
            raise self._error(f"Import namespace {alias!r} conflicts with a builtin", alias_token)

        info = self._shared.modules.get(module_filename)
        if info is None or not info.finalized:
            if module_filename in self._shared.loading_modules:
                raise self._error_at(f"Import cycle involving {module_filename}", path_token.pos)
            self._shared.loading_modules.add(module_filename)
            self._imported.add(module_filename)
            try:
                module_compiler = Compiler(
                    module_source,
                    filename=module_filename,
                    resolver=self._resolver,
                    imported=self._imported,
                    module_mode=True,
                    module_name=_module_name_from_import(path_token.text, module_filename),
                    shared=self._shared,
                )
                module_compiler.compile()
            finally:
                self._shared.loading_modules.discard(module_filename)
            info = self._shared.modules[module_filename]

        self._namespaces[alias] = info
        self._module_imports.append(ModuleImportDef(alias, path_token.text, info.name, info.name))

    def _struct_definition(self, *, exported: bool) -> None:
        self._eat(TokenKind.STRUCT)
        name_token = self._current
        name = name_token.text
        self._eat(TokenKind.IDENT)
        if name in self._namespaces:
            raise self._error(f"Struct {name!r} conflicts with an imported namespace", name_token)
        if name in self._local_struct_indexes:
            raise self._error(f"Struct {name!r} is already defined", name_token)
        if name in self._local_function_indexes or name in _BUILTIN_INDEXES:
            raise self._error(f"Struct {name!r} conflicts with an existing callable", name_token)

        fields: list[str] = []
        seen: set[str] = set()
        self._eat(TokenKind.LBRACE)
        if self._current.kind != TokenKind.RBRACE:
            while True:
                field_token = self._current
                field = field_token.text
                self._eat(TokenKind.IDENT)
                if field in seen:
                    raise self._error(f"Duplicate struct field {field!r}", field_token)
                seen.add(field)
                fields.append(field)
                self._intern_field(field)
                if self._current.kind != TokenKind.COMMA:
                    break
                self._eat(TokenKind.COMMA)
        self._eat(TokenKind.RBRACE)

        full_name = self._qualified_declaration_name(name)
        if full_name in self._struct_indexes:
            raise self._error(f"Struct {full_name!r} is already defined", name_token)
        struct_index = len(self._structs)
        self._struct_indexes[full_name] = struct_index
        self._local_struct_indexes[name] = struct_index
        self._structs.append(StructDef(full_name, tuple(fields)))
        if self._module_info is not None:
            self._module_info.all_structs.add(name)
            if exported:
                self._module_info.struct_exports[name] = struct_index

    def _function_definition(self, *, exported: bool) -> None:
        self._eat(TokenKind.FN)
        name = self._current.text
        name_pos = self._current.pos
        name_token = self._current
        self._eat(TokenKind.IDENT)
        if name in self._namespaces:
            raise self._error(f"Function {name!r} conflicts with an imported namespace", name_token)
        if name in self._local_function_indexes:
            raise self._error(f"Function {name!r} is already defined", name_token)
        if name in self._local_struct_indexes or name in _BUILTIN_INDEXES:
            raise self._error(f"Function {name!r} conflicts with an existing callable", name_token)
        full_name = self._qualified_declaration_name(name)
        if full_name in self._function_indexes:
            raise self._error(f"Function {full_name!r} is already defined", name_token)
        function_index = len(self._functions)
        self._function_indexes[full_name] = function_index
        self._local_function_indexes[name] = function_index
        if self._module_info is not None:
            self._module_info.all_functions.add(name)
            if exported:
                self._module_info.function_exports[name] = function_index

        function_symbols = SymbolTable()
        self._eat(TokenKind.LPAREN)
        param_count = 0
        if self._current.kind != TokenKind.RPAREN:
            while True:
                param_name = self._current.text
                param_pos = self._current.pos
                param_token = self._current
                self._eat(TokenKind.IDENT)
                slot = function_symbols.define_current(param_name)
                if slot is None:
                    raise self._error(f"Duplicate parameter {param_name!r}", param_token)
                assert int(slot) == param_count
                param_count += 1
                if self._current.kind != TokenKind.COMMA:
                    break
                self._eat(TokenKind.COMMA)
        self._eat(TokenKind.RPAREN)

        previous_symbols = self._symbols
        previous_code = self._code
        previous_in_function = self._in_function
        self._symbols = function_symbols
        self._code = []
        self._in_function = True
        try:
            self._block()
            self._emit(Op.PUSH_INT, 0)
            self._emit(Op.RETURN)
            function = Function(
                full_name,
                param_count,
                tuple(self._code),
                self._symbols.slot_count,
                self._symbols.names,
            )
        finally:
            self._symbols = previous_symbols
            self._code = previous_code
            self._in_function = previous_in_function

        self._functions.append(function)
        LOGGER.debug(
            "compiled function",
            extra={"name": name, "index": function_index, "params": param_count},
        )

    def _qualified_declaration_name(self, name: str) -> str:
        if self._module_info is None:
            return name
        return f"{self._module_info.name}.{name}"

    def _block(self) -> None:
        self._eat(TokenKind.LBRACE)
        self._symbols.enter_scope()
        try:
            while self._current.kind != TokenKind.RBRACE:
                if self._current.kind == TokenKind.EOF:
                    raise self._error("Unterminated block", self._current)
                self._statement()
            self._eat(TokenKind.RBRACE)
        finally:
            self._symbols.exit_scope()

    def _expression(self) -> None:
        self._binary_level(self._additive, _COMPARISON_OPS)

    def _additive(self) -> None:
        self._binary_level(self._term, _ADDITIVE_OPS)

    def _term(self) -> None:
        self._binary_level(self._factor, _TERM_OPS)

    def _binary_level(
        self, parse_operand: Callable[[], None], operators: dict[TokenKind, Op]
    ) -> None:
        parse_operand()
        while self._current.kind in operators:
            op = self._current.kind
            self._eat(op)
            parse_operand()
            self._emit(operators[op])

    def _factor(self) -> None:
        self._primary()
        while True:
            if self._current.kind == TokenKind.LBRACKET:
                self._eat(TokenKind.LBRACKET)
                self._expression()
                self._eat(TokenKind.RBRACKET)
                self._emit(Op.INDEX)
                continue
            if self._current.kind == TokenKind.DOT:
                self._eat(TokenKind.DOT)
                field = self._current.text
                self._eat(TokenKind.IDENT)
                self._emit(Op.GET_FIELD, self._intern_field(field))
                continue
            break

    def _primary(self) -> None:
        token = self._current
        kind = token.kind

        if kind == TokenKind.INT:
            self._eat(TokenKind.INT)
            self._emit(Op.PUSH_INT, int(token.text))
            return

        if kind == TokenKind.STRING:
            self._eat(TokenKind.STRING)
            self._emit(Op.PUSH_STRING, self._intern_string(token.text))
            return

        if kind == TokenKind.NULL:
            self._eat(TokenKind.NULL)
            self._emit(Op.PUSH_NULL)
            return

        if kind == TokenKind.LBRACKET:
            self._eat(TokenKind.LBRACKET)
            count = 0
            if self._current.kind != TokenKind.RBRACKET:
                while True:
                    self._expression()
                    count += 1
                    if self._current.kind != TokenKind.COMMA:
                        break
                    self._eat(TokenKind.COMMA)
            self._eat(TokenKind.RBRACKET)
            self._emit(Op.MAKE_ARRAY, count)
            return

        if kind == TokenKind.IDENT:
            if self._is_qualified_call():
                namespace = token.text
                self._eat(TokenKind.IDENT)
                self._eat(TokenKind.DOT)
                member_token = self._current
                member = member_token.text
                self._eat(TokenKind.IDENT)
                self._qualified_call_expression(namespace, member, token.pos, member_token.pos)
                return
            self._eat(TokenKind.IDENT)
            if self._current.kind == TokenKind.LPAREN:
                self._call_expression(token.text, token.pos)
            else:
                slot = self._get_slot(token)
                self._emit(Op.LOAD, int(slot))
            return

        if kind == TokenKind.LPAREN:
            self._eat(TokenKind.LPAREN)
            self._expression()
            self._eat(TokenKind.RPAREN)
            return

        if kind == TokenKind.MINUS:
            self._eat(TokenKind.MINUS)
            self._factor()
            self._emit(Op.NEG)
            return

        if kind == TokenKind.UNSAFE:
            self._eat(TokenKind.UNSAFE)
            self._unsafe_depth += 1
            try:
                self._factor()
            finally:
                self._unsafe_depth -= 1
            return

        raise self._error("Expected expression", token)

    def _is_qualified_call(self) -> bool:
        return (
            self._current.kind == TokenKind.IDENT
            and self._index + 3 < len(self._tokens)
            and self._tokens[self._index + 1].kind == TokenKind.DOT
            and self._tokens[self._index + 2].kind == TokenKind.IDENT
            and self._tokens[self._index + 3].kind == TokenKind.LPAREN
        )

    def _call_expression(self, name: str, pos: int) -> None:
        struct_index = self._local_struct_indexes.get(name)
        if struct_index is not None:
            self._constructor_call(name, struct_index)
            return

        builtin_index = _BUILTIN_INDEXES.get(name)
        if builtin_index is not None:
            self._builtin_call(name, builtin_index, pos)
            return

        function_index = self._local_function_indexes.get(name)
        if function_index is None:
            raise self._error_at(f"Undefined function or constructor {name!r}", pos)
        self._eat(TokenKind.LPAREN)
        arg_count = 0
        if self._current.kind != TokenKind.RPAREN:
            while True:
                self._expression()
                arg_count += 1
                if self._current.kind != TokenKind.COMMA:
                    break
                self._eat(TokenKind.COMMA)
        self._eat(TokenKind.RPAREN)
        self._emit(Op.CALL, function_index, arg_count)

    def _qualified_call_expression(
        self, namespace: str, member: str, namespace_pos: int, member_pos: int
    ) -> None:
        info = self._namespaces.get(namespace)
        if info is None:
            raise self._error_at(f"Unknown module namespace {namespace!r}", namespace_pos)

        struct_index = info.struct_exports.get(member)
        if struct_index is not None:
            self._constructor_call(f"{namespace}.{member}", struct_index)
            return

        function_index = info.function_exports.get(member)
        if function_index is not None:
            self._eat(TokenKind.LPAREN)
            arg_count = 0
            if self._current.kind != TokenKind.RPAREN:
                while True:
                    self._expression()
                    arg_count += 1
                    if self._current.kind != TokenKind.COMMA:
                        break
                    self._eat(TokenKind.COMMA)
            self._eat(TokenKind.RPAREN)
            self._emit(Op.CALL, function_index, arg_count)
            return

        if member in info.all_functions or member in info.all_structs:
            raise self._error_at(
                f"Module member {namespace}.{member} is not exported", member_pos
            )
        raise self._error_at(f"Module {namespace!r} has no exported member {member!r}", member_pos)

    def _constructor_call(self, name: str, struct_index: int) -> None:
        struct = self._structs[struct_index]
        self._eat(TokenKind.LPAREN)
        arg_count = 0
        if self._current.kind != TokenKind.RPAREN:
            while True:
                self._expression()
                arg_count += 1
                if self._current.kind != TokenKind.COMMA:
                    break
                self._eat(TokenKind.COMMA)
        self._eat(TokenKind.RPAREN)
        if arg_count != len(struct.fields):
            raise self._error_at(
                f"Struct {name!r} expects {len(struct.fields)} field value(s), got {arg_count}",
                self._current.pos,
            )
        self._emit(Op.MAKE_STRUCT, struct_index, arg_count)

    def _builtin_call(self, name: str, builtin_index: int, pos: int) -> None:
        builtin = _BUILTINS[builtin_index]
        self._eat(TokenKind.LPAREN)
        arg_count = 0
        if self._current.kind != TokenKind.RPAREN:
            while True:
                self._expression()
                arg_count += 1
                if self._current.kind != TokenKind.COMMA:
                    break
                self._eat(TokenKind.COMMA)
        self._eat(TokenKind.RPAREN)
        if not builtin.min_args <= arg_count <= builtin.max_args:
            if builtin.min_args == builtin.max_args:
                expected = str(builtin.min_args)
            else:
                expected = f"{builtin.min_args}..{builtin.max_args}"
            raise self._error_at(
                f"Builtin {name!r} expects {expected} argument(s), got {arg_count}",
                self._current.pos,
            )
        if builtin.requires_unsafe and self._unsafe_depth <= 0:
            raise self._error_at(
                f"Builtin {name!r} requires unsafe dereference syntax",
                pos,
            )
        self._emit(Op.BUILTIN, builtin_index, arg_count)

    def _eat(self, kind: TokenKind) -> None:
        if self._current.kind != kind:
            raise self._error(f"Expected {kind.name}, got {self._current.kind.name}", self._current)
        self._index += 1
        self._current = self._tokens[self._index]

    def _peek_kind(self, offset: int) -> TokenKind | None:
        index = self._index + offset
        if index >= len(self._tokens):
            return None
        return self._tokens[index].kind

    def _emit(self, op: Op, arg: int = 0, arg2: int = 0) -> None:
        self._code.append(Instr(op, arg, arg2))

    def _emit_placeholder(self, op: Op) -> int:
        index = len(self._code)
        self._code.append(Instr(op, -1))
        return index

    def _patch(self, index: int, arg: int) -> None:
        instr = self._code[index]
        self._code[index] = Instr(instr.op, arg, instr.arg2)

    def _get_slot(self, token: Token) -> Slot:
        try:
            return self._symbols.get(token.text, token.pos)
        except CompileError:
            raise self._error(f"Undefined variable {token.text!r}", token) from None

    def _intern_string(self, text: str) -> int:
        existing = self._string_indexes.get(text)
        if existing is not None:
            return existing
        index = len(self._strings)
        self._string_indexes[text] = index
        self._strings.append(text)
        return index

    def _intern_field(self, name: str) -> int:
        existing = self._field_indexes.get(name)
        if existing is not None:
            return existing
        index = len(self._fields)
        self._field_indexes[name] = index
        self._fields.append(name)
        return index

    def _error(self, message: str, token: Token) -> CompileError:
        return CompileError(self._source_map.format(message, token.pos, token.end))

    def _error_at(self, message: str, pos: int) -> CompileError:
        return CompileError(self._source_map.format(message, pos, pos + 1))


# ---------------------------------------------------------------------------
# Stack effects for each opcode.  Used by BytecodeVerifier and kept as a
# module-level constant so both components share the same source of truth.
# ---------------------------------------------------------------------------
_STACK_EFFECTS: Final[dict[Op, int]] = {
    Op.PUSH_INT: 1,
    Op.PUSH_STRING: 1,
    Op.PUSH_NULL: 1,
    Op.LOAD: 1,
    Op.STORE: -1,
    Op.ADD: -1,
    Op.SUB: -1,
    Op.MUL: -1,
    Op.DIV: -1,
    Op.NEG: 0,
    Op.PRINT: -1,
    Op.LT: -1,
    Op.LTE: -1,
    Op.GT: -1,
    Op.GTE: -1,
    Op.EQ: -1,
    Op.NE: -1,
    Op.INDEX: -1,
    Op.GET_FIELD: 0,
    Op.SET_INDEX: -3,
    Op.SET_FIELD: -2,
}

# ---------------------------------------------------------------------------
# Dispatch tables replacing if/elif chains.  All are Final so CPython can
# treat them as module constants; dict lookup is O(1) vs O(n) linear scan.
# ---------------------------------------------------------------------------

_COMPARE_FUNCS: Final[dict[Op, Callable[[int, int], int]]] = {
    Op.LT:  lambda a, b: 1 if a < b else 0,
    Op.LTE: lambda a, b: 1 if a <= b else 0,
    Op.GT:  lambda a, b: 1 if a > b else 0,
    Op.GTE: lambda a, b: 1 if a >= b else 0,
    Op.EQ:  lambda a, b: 1 if a == b else 0,
    Op.NE:  lambda a, b: 1 if a != b else 0,
}

# DIV is absent — div-by-zero guard must stay inline.
_FOLD_BINOPS: Final[dict[Op, Callable[[int, int], int]]] = {
    Op.ADD: lambda a, b: a + b,
    Op.SUB: lambda a, b: a - b,
    Op.MUL: lambda a, b: a * b,
    Op.LT:  lambda a, b: 1 if a < b else 0,
    Op.LTE: lambda a, b: 1 if a <= b else 0,
    Op.GT:  lambda a, b: 1 if a > b else 0,
    Op.GTE: lambda a, b: 1 if a >= b else 0,
    Op.EQ:  lambda a, b: 1 if a == b else 0,
    Op.NE:  lambda a, b: 1 if a != b else 0,
}

_CMP_PYTHON_OP: Final[dict[Op, str]] = {
    Op.LT:  "<",
    Op.LTE: "<=",
    Op.GT:  ">",
    Op.GTE: ">=",
    Op.EQ:  "==",
    Op.NE:  "!=",
}


class BytecodeVerifier:
    """Tiny CFG stack checker for compiler-generated bytecode."""

    @staticmethod
    def verify(program: Program) -> None:
        BytecodeVerifier._verify_chunk(
            "main",
            program.code,
            program.slot_count,
            program.functions,
            program.strings,
            program.structs,
            program.fields,
            Op.HALT,
        )
        for index, function in enumerate(program.functions):
            BytecodeVerifier._verify_chunk(
                f"function {function.name!r} (index {index})",
                function.code,
                function.slot_count,
                program.functions,
                program.strings,
                program.structs,
                program.fields,
                Op.RETURN,
            )

    @staticmethod
    def _verify_chunk(
        chunk_name: str,
        code: tuple[Instr, ...],
        slot_count: int,
        functions: tuple[Function, ...],
        strings: tuple[str, ...],
        structs: tuple[StructDef, ...],
        fields: tuple[str, ...],
        final_op: Op,
    ) -> None:
        if not code or code[-1].op != final_op:
            got = "nothing" if not code else code[-1].op.name
            raise CompileError(f"Verifier: {chunk_name} must end with {final_op.name}, got {got}")

        seen: dict[int, int] = {}
        todo: list[tuple[int, int]] = []

        def visit(pc: int, depth: int, origin: int) -> None:
            if pc < 0 or pc >= len(code):
                raise CompileError(
                    f"Verifier: instruction {origin} in {chunk_name} targets {pc}"
                )
            old_depth = seen.get(pc)
            if old_depth is not None:
                if old_depth != depth:
                    raise CompileError(
                        f"Verifier: stack depth mismatch at instruction {pc} "
                        f"in {chunk_name}: {old_depth} vs {depth}"
                    )
                return
            seen[pc] = depth
            todo.append((pc, depth))

        def next_depth(pc: int, depth: int, delta: int) -> int:
            depth += delta
            if depth < 0:
                raise CompileError(
                    f"Verifier: stack underflow at instruction {pc} in {chunk_name}"
                )
            return depth

        visit(0, 0, 0)
        while todo:
            pc, depth = todo.pop()
            instr = code[pc]
            op, arg, arg2 = instr.op, instr.arg, instr.arg2

            if op in (Op.LOAD, Op.STORE) and not 0 <= arg < slot_count:
                raise CompileError(
                    f"Verifier: invalid slot {arg} at instruction {pc} in {chunk_name}"
                )
            if op == Op.PUSH_STRING and not 0 <= arg < len(strings):
                raise CompileError(
                    f"Verifier: invalid string index {arg} at instruction {pc} in {chunk_name}"
                )
            if op in (Op.GET_FIELD, Op.SET_FIELD) and not 0 <= arg < len(fields):
                raise CompileError(
                    f"Verifier: invalid field index {arg} at instruction {pc} in {chunk_name}"
                )

            if op == Op.JUMP:
                visit(arg, depth, pc)
            elif op == Op.JUMP_IF_ZERO:
                depth = next_depth(pc, depth, -1)
                visit(arg, depth, pc)
                visit(pc + 1, depth, pc)
            elif op == Op.CALL:
                if not 0 <= arg < len(functions):
                    raise CompileError(
                        f"Verifier: invalid function index {arg} at instruction {pc} "
                        f"in {chunk_name}"
                    )
                expected = functions[arg].param_count
                if arg2 != expected:
                    raise CompileError(
                        f"Function {functions[arg].name!r} expects {expected} argument(s), "
                        f"got {arg2} at instruction {pc} in {chunk_name}"
                    )
                visit(pc + 1, next_depth(pc, depth, 1 - arg2), pc)
            elif op == Op.MAKE_ARRAY:
                if arg < 0:
                    raise CompileError(
                        f"Verifier: negative array arity {arg} at instruction {pc} "
                        f"in {chunk_name}"
                    )
                visit(pc + 1, next_depth(pc, depth, 1 - arg), pc)
            elif op == Op.MAKE_STRUCT:
                if not 0 <= arg < len(structs):
                    raise CompileError(
                        f"Verifier: invalid struct index {arg} at instruction {pc} "
                        f"in {chunk_name}"
                    )
                expected = len(structs[arg].fields)
                if arg2 != expected:
                    raise CompileError(
                        f"Struct {structs[arg].name!r} expects {expected} field value(s), "
                        f"got {arg2} at instruction {pc} in {chunk_name}"
                    )
                visit(pc + 1, next_depth(pc, depth, 1 - arg2), pc)
            elif op == Op.BUILTIN:
                if not 0 <= arg < len(_BUILTINS):
                    raise CompileError(
                        f"Verifier: invalid builtin index {arg} at instruction {pc} "
                        f"in {chunk_name}"
                    )
                builtin = _BUILTINS[arg]
                if not builtin.min_args <= arg2 <= builtin.max_args:
                    raise CompileError(
                        f"Builtin {builtin.name!r} expects {builtin.min_args}.."
                        f"{builtin.max_args} argument(s), got {arg2} at instruction {pc} "
                        f"in {chunk_name}"
                    )
                visit(pc + 1, next_depth(pc, depth, 1 - arg2), pc)
            elif op == Op.RETURN:
                if depth != 1:
                    raise CompileError(
                        f"Verifier: RETURN in {chunk_name} requires one value, got {depth}"
                    )
            elif op == Op.HALT:
                if depth != 0:
                    raise CompileError(
                        f"Verifier: HALT in {chunk_name} requires empty stack, got {depth}"
                    )
            else:
                effect = _STACK_EFFECTS.get(op)
                if effect is None:
                    raise CompileError(
                        f"Verifier: unknown opcode {op!r} at index {pc} in {chunk_name}"
                    )
                visit(pc + 1, next_depth(pc, depth, effect), pc)


class PeepholeOptimizer:
    """
    Constant-folding peephole optimizer over flat bytecode.

    Folds these patterns inside branch-free bytecode chunks:
        PUSH_INT a, NEG              ->  PUSH_INT (-a)
        PUSH_INT a, PUSH_INT b, ADD  ->  PUSH_INT (a + b)
        PUSH_INT a, PUSH_INT b, SUB  ->  PUSH_INT (a - b)
        PUSH_INT a, PUSH_INT b, MUL  ->  PUSH_INT (a * b)
        PUSH_INT a, PUSH_INT b, DIV  ->  PUSH_INT (a // b)  [skipped if b==0]
        PUSH_INT a, PUSH_INT b, <cmp> -> PUSH_INT 0/1

    Runs until convergence so cascading folds resolve in one compilation.
    Each pass is O(n); worst-case passes = fold-chain length (bounded by
    the original instruction count).

    Division by zero at fold time is left intact so the runtime error fires
    as the user would expect — we don't silently swallow it at compile time.
    """

    @staticmethod
    def optimize(program: Program) -> Program:
        functions = tuple(
            Function(
                function.name,
                function.param_count,
                PeepholeOptimizer._optimize_code(function.code),
                function.slot_count,
                function.names,
            )
            for function in program.functions
        )
        return Program(
            PeepholeOptimizer._optimize_code(program.code),
            program.slot_count,
            program.names,
            functions,
            program.strings,
            program.structs,
            program.fields,
            program.modules,
        )

    @staticmethod
    def _optimize_code(original_code: tuple[Instr, ...]) -> tuple[Instr, ...]:
        if any(instr.op in (Op.JUMP, Op.JUMP_IF_ZERO) for instr in original_code):
            return original_code

        code = list(original_code)
        changed = True
        while changed:
            changed = False
            out: list[Instr] = []
            i = 0
            while i < len(code):
                # Pattern: PUSH_INT a, NEG  ->  PUSH_INT (-a)
                if (
                    i + 1 < len(code)
                    and code[i].op == Op.PUSH_INT
                    and code[i + 1].op == Op.NEG
                ):
                    out.append(Instr(Op.PUSH_INT, -code[i].arg))
                    i += 2
                    changed = True
                    continue

                # Pattern: PUSH_INT a, PUSH_INT b, <binop>  ->  PUSH_INT result
                if (
                    i + 2 < len(code)
                    and code[i].op == Op.PUSH_INT
                    and code[i + 1].op == Op.PUSH_INT
                ):
                    fold_op = code[i + 2].op
                    a = code[i].arg
                    b = code[i + 1].arg

                    fold_fn = _FOLD_BINOPS.get(fold_op)
                    if fold_fn is not None:
                        out.append(Instr(Op.PUSH_INT, fold_fn(a, b)))
                        i += 3
                        changed = True
                        continue

                    if fold_op == Op.DIV:
                        if b == 0:
                            out.append(code[i])
                            i += 1
                            continue
                        out.append(Instr(Op.PUSH_INT, a // b))
                        i += 3
                        changed = True
                        continue

                out.append(code[i])
                i += 1
            code = out

        return tuple(code)


@dataclass(frozen=True, slots=True)
class HeapRef:
    address: int
    generation: int


@dataclass(frozen=True, slots=True)
class RawPointer:
    address: int
    kind: str = "object"
    index: int = 0
    field: str = ""
    generation: int = 0
    cast: str = ""


@dataclass(slots=True)
class HeapObject:
    kind: str
    value: object
    type_name: str = ""


Value = int | HeapRef | RawPointer


class TinyHeap:
    """Explicit heap for arrays, structs, strings, and pointer-like cells."""

    __slots__ = ("_objects", "_free", "_generations")

    def __init__(self) -> None:
        self._objects: list[HeapObject | None] = [None]
        self._free: list[int] = []
        self._generations: list[int] = [0]

    def alloc(self, obj: HeapObject) -> HeapRef:
        if self._free:
            address = self._free.pop()
            self._generations[address] += 1
            self._objects[address] = obj
        else:
            address = len(self._objects)
            self._objects.append(obj)
            self._generations.append(1)
        return HeapRef(address, self._generations[address])

    def alloc_string(self, text: str) -> HeapRef:
        return self.alloc(HeapObject("string", text))

    def alloc_array(self, values: Iterable[Value]) -> HeapRef:
        return self.alloc(HeapObject("array", list(values)))

    def alloc_buffer(self, size: int) -> HeapRef:
        return self.alloc(HeapObject("buffer", bytearray(size)))

    def alloc_struct(self, type_name: str, fields: dict[str, Value]) -> HeapRef:
        return self.alloc(HeapObject("struct", dict(fields), type_name))

    def alloc_cell(self, value: Value) -> HeapRef:
        return self.alloc(HeapObject("cell", value))

    def alloc_map(self, entries: Iterable[tuple[Value, Value]]) -> HeapRef:
        return self.alloc(HeapObject("map", [(k, v) for k, v in entries]))

    def get(self, ref: Value) -> HeapObject:
        if not isinstance(ref, HeapRef):
            raise RuntimeTinyOneError("Expected heap pointer")
        return self.get_address(ref.address, ref.generation)

    def ref_at(self, address: int) -> HeapRef:
        return HeapRef(address, self.current_generation(address))

    def current_generation(self, address: int) -> int:
        self._current_object(address)
        return self._generations[address]

    def get_address(self, address: int, generation: int = 0) -> HeapObject:
        obj = self._current_object(address)
        if generation != 0 and self._generations[address] != generation:
            raise RuntimeTinyOneError(f"Stale heap pointer {address}")
        return obj

    def _current_object(self, address: int) -> HeapObject:
        if address <= 0 or address >= len(self._objects):
            raise RuntimeTinyOneError(f"Invalid heap pointer {address}")
        obj = self._objects[address]
        if obj is None:
            raise RuntimeTinyOneError(f"Use after free for heap pointer {address}")
        return obj

    def free(self, ref: Value) -> None:
        obj = self.get(ref)
        address = ref.address
        self._objects[address] = None
        self._free.append(address)
        obj.value = None


class TinyRuntimeContext:
    """Runtime state shared by stack frames: heap plus deterministic input."""

    __slots__ = (
        "heap",
        "_inputs",
        "_input_index",
        "io_stdout",
        "io_stderr",
        "sys_args",
        "sys_env",
    )

    def __init__(
        self,
        inputs: Iterable[object] | None = None,
        *,
        sys_args: Iterable[str] | None = None,
        sys_env: dict[str, str] | None = None,
    ) -> None:
        self.heap = TinyHeap()
        self._inputs = [str(value) for value in (inputs or ())]
        self._input_index = 0
        # Phase 2 stdlib-bridge state:
        # io_stdout/io_stderr collect bytes written via io_write/io_writeln
        # (separate from the real Python stdout); sys_args/sys_env are
        # deterministic, never touch the host process environment unless the
        # caller passes them in explicitly.
        self.io_stdout: list[str] = []
        self.io_stderr: list[str] = []
        self.sys_args: list[str] = list(sys_args or ())
        self.sys_env: dict[str, str] = dict(sys_env or {})

    def read_raw(self) -> str:
        if self._input_index >= len(self._inputs):
            raise RuntimeTinyOneError("Input exhausted")
        value = self._inputs[self._input_index]
        self._input_index += 1
        return value


def runtime_expect_int(value: Value, operation: str) -> int:
    if isinstance(value, int):
        return value
    raise RuntimeTinyOneError(f"{operation} expects integer operands")


def runtime_add(lhs: Value, rhs: Value) -> int:
    return runtime_expect_int(lhs, "Addition") + runtime_expect_int(rhs, "Addition")


def runtime_sub(lhs: Value, rhs: Value) -> int:
    return runtime_expect_int(lhs, "Subtraction") - runtime_expect_int(rhs, "Subtraction")


def runtime_mul(lhs: Value, rhs: Value) -> int:
    return runtime_expect_int(lhs, "Multiplication") * runtime_expect_int(rhs, "Multiplication")


def checked_div(lhs: Value, rhs: Value) -> int:
    lhs_int = runtime_expect_int(lhs, "Division")
    rhs_int = runtime_expect_int(rhs, "Division")
    if rhs_int == 0:
        raise RuntimeTinyOneError("Division by zero")
    return lhs_int // rhs_int


def runtime_neg(value: Value) -> int:
    return -runtime_expect_int(value, "Negation")


def runtime_compare(op: Op, lhs: Value, rhs: Value) -> int:
    lhs_int = runtime_expect_int(lhs, op.name)
    rhs_int = runtime_expect_int(rhs, op.name)
    fn = _COMPARE_FUNCS.get(op)
    if fn is None:
        raise RuntimeTinyOneError(f"Unsupported comparison opcode {op!r}")
    return fn(lhs_int, rhs_int)


def runtime_is_false(value: Value) -> bool:
    return (isinstance(value, int) and value == 0) or runtime_is_null(value)


def runtime_make_array(context: TinyRuntimeContext, values: Iterable[Value]) -> HeapRef:
    return context.heap.alloc_array(values)


def runtime_index(context: TinyRuntimeContext, container: Value, index: Value) -> Value:
    index_int = runtime_expect_int(index, "Index")
    obj = context.heap.get(container)
    if obj.kind == "array":
        values = obj.value
        if not isinstance(values, list):
            raise RuntimeTinyOneError("Corrupt array object")
        if index_int < 0 or index_int >= len(values):
            raise RuntimeTinyOneError(f"Array index {index_int} out of bounds")
        return values[index_int]
    if obj.kind == "string":
        text = obj.value
        if not isinstance(text, str):
            raise RuntimeTinyOneError("Corrupt string object")
        if index_int < 0 or index_int >= len(text):
            raise RuntimeTinyOneError(f"String index {index_int} out of bounds")
        return context.heap.alloc_string(text[index_int])
    raise RuntimeTinyOneError(f"Cannot index {obj.kind}")


def runtime_set_index(
    context: TinyRuntimeContext, container: Value, index: Value, value: Value
) -> None:
    index_int = runtime_expect_int(index, "Index")
    obj = context.heap.get(container)
    if obj.kind != "array":
        raise RuntimeTinyOneError(f"Cannot assign index on {obj.kind}")
    values = obj.value
    if not isinstance(values, list):
        raise RuntimeTinyOneError("Corrupt array object")
    if index_int < 0 or index_int >= len(values):
        raise RuntimeTinyOneError(f"Array index {index_int} out of bounds")
    values[index_int] = value


def runtime_make_struct(
    context: TinyRuntimeContext, type_name: str, field_names: tuple[str, ...], values: Iterable[Value]
) -> HeapRef:
    return context.heap.alloc_struct(type_name, dict(zip(field_names, values)))


def runtime_get_field(context: TinyRuntimeContext, target: Value, field: str) -> Value:
    obj = context.heap.get(target)
    if obj.kind != "struct":
        raise RuntimeTinyOneError(f"Cannot read field {field!r} from {obj.kind}")
    fields = obj.value
    if not isinstance(fields, dict):
        raise RuntimeTinyOneError("Corrupt struct object")
    if field not in fields:
        raise RuntimeTinyOneError(f"Unknown field {field!r} on struct {obj.type_name!r}")
    return fields[field]


def runtime_set_field(
    context: TinyRuntimeContext, target: Value, field: str, value: Value
) -> None:
    obj = context.heap.get(target)
    if obj.kind != "struct":
        raise RuntimeTinyOneError(f"Cannot write field {field!r} on {obj.kind}")
    fields = obj.value
    if not isinstance(fields, dict):
        raise RuntimeTinyOneError("Corrupt struct object")
    if field not in fields:
        raise RuntimeTinyOneError(f"Unknown field {field!r} on struct {obj.type_name!r}")
    fields[field] = value


def runtime_expect_string(context: TinyRuntimeContext, value: Value, operation: str) -> str:
    obj = context.heap.get(value)
    if obj.kind != "string" or not isinstance(obj.value, str):
        raise RuntimeTinyOneError(f"{operation} expects a string")
    return obj.value


def runtime_null() -> RawPointer:
    return RawPointer(0, "null")


def runtime_is_null(value: Value) -> bool:
    return isinstance(value, RawPointer) and value.kind == "null" and value.address == 0


def runtime_expect_pointer(value: Value, operation: str) -> RawPointer:
    if not isinstance(value, RawPointer):
        raise RuntimeTinyOneError(f"{operation} expects a raw pointer")
    return value


def runtime_validate_pointer_base(
    context: TinyRuntimeContext, pointer: RawPointer, operation: str
) -> None:
    if runtime_is_null(pointer):
        return
    if pointer.kind in ("object", "array", "buffer", "field"):
        context.heap.get_address(pointer.address, pointer.generation)
        return
    raise RuntimeTinyOneError(f"{operation} got unknown raw pointer kind {pointer.kind!r}")


def runtime_pointer_identity(pointer: RawPointer) -> tuple[int, int, str, int, str]:
    if runtime_is_null(pointer):
        return (0, 0, "null", 0, "")
    return (pointer.address, pointer.generation, pointer.kind, pointer.index, pointer.field)


def runtime_make_pointer(context: TinyRuntimeContext, args: list[Value]) -> RawPointer:
    if len(args) == 1:
        target = args[0]
        if isinstance(target, RawPointer):
            return target
        if not isinstance(target, HeapRef):
            raise RuntimeTinyOneError("ptr() expects a heap value or pointer")
        context.heap.get(target)
        return RawPointer(target.address, generation=target.generation)

    target, index = args
    if not isinstance(target, HeapRef):
        raise RuntimeTinyOneError("ptr(value, index) expects an array or buffer heap value")
    obj = context.heap.get(target)
    index_int = runtime_expect_int(index, "ptr index")
    if obj.kind == "array":
        return RawPointer(target.address, "array", index_int, generation=target.generation)
    if obj.kind == "buffer":
        return RawPointer(target.address, "buffer", index_int, generation=target.generation)
    raise RuntimeTinyOneError("ptr(value, index) expects an array or buffer heap value")


def runtime_make_field_pointer(
    context: TinyRuntimeContext, target: Value, field_value: Value
) -> RawPointer:
    if not isinstance(target, HeapRef):
        raise RuntimeTinyOneError("fieldptr() expects a struct heap value")
    obj = context.heap.get(target)
    if obj.kind != "struct":
        raise RuntimeTinyOneError("fieldptr() expects a struct heap value")
    field = runtime_expect_string(context, field_value, "fieldptr")
    fields = obj.value
    if not isinstance(fields, dict):
        raise RuntimeTinyOneError("Corrupt struct object")
    if field not in fields:
        raise RuntimeTinyOneError(f"Unknown field {field!r} on struct {obj.type_name!r}")
    return RawPointer(target.address, "field", field=field, generation=target.generation)


def runtime_pointer_address(context: TinyRuntimeContext, value: Value) -> int:
    if isinstance(value, RawPointer):
        runtime_validate_pointer_base(context, value, "ptr_addr")
        return value.address
    if isinstance(value, HeapRef):
        context.heap.get(value)
        return value.address
    raise RuntimeTinyOneError("ptr_addr() expects a heap value or raw pointer")


def runtime_pointer_at(context: TinyRuntimeContext, address: Value) -> RawPointer:
    address_int = runtime_expect_int(address, "ptr_at")
    generation = context.heap.current_generation(address_int)
    return RawPointer(address_int, generation=generation)


def runtime_pointer_add(context: TinyRuntimeContext, pointer: Value, offset: Value) -> RawPointer:
    pointer = runtime_expect_pointer(pointer, "ptr_add")
    runtime_validate_pointer_base(context, pointer, "ptr_add")
    if runtime_is_null(pointer):
        raise RuntimeTinyOneError("Cannot apply pointer arithmetic to null")
    offset_int = runtime_expect_int(offset, "ptr_add")
    if pointer.kind == "object":
        if offset_int != 0:
            raise RuntimeTinyOneError("Object pointer arithmetic requires an array or buffer pointer")
        return pointer
    if pointer.kind == "array":
        return RawPointer(
            pointer.address,
            "array",
            pointer.index + offset_int,
            generation=pointer.generation,
            cast=pointer.cast,
        )
    if pointer.kind == "buffer":
        return RawPointer(
            pointer.address,
            "buffer",
            pointer.index + offset_int,
            generation=pointer.generation,
            cast=pointer.cast,
        )
    if pointer.kind == "field":
        raise RuntimeTinyOneError("Cannot apply pointer arithmetic to field pointers")
    raise RuntimeTinyOneError(f"Unknown raw pointer kind {pointer.kind!r}")


def runtime_pointer_load(context: TinyRuntimeContext, pointer: Value) -> Value:
    pointer = runtime_expect_pointer(pointer, "ptr_load")
    if runtime_is_null(pointer):
        raise RuntimeTinyOneError("Cannot load through null")
    if pointer.kind == "object":
        obj = context.heap.get_address(pointer.address, pointer.generation)
        if obj.kind == "cell":
            return obj.value if isinstance(obj.value, (int, HeapRef, RawPointer)) else 0
        return context.heap.ref_at(pointer.address)
    if pointer.kind == "array":
        obj = context.heap.get_address(pointer.address, pointer.generation)
        if obj.kind != "array":
            raise RuntimeTinyOneError("Array pointer no longer points at an array")
        values = obj.value
        if not isinstance(values, list):
            raise RuntimeTinyOneError("Corrupt array object")
        if pointer.index < 0 or pointer.index >= len(values):
            raise RuntimeTinyOneError(f"Array pointer index {pointer.index} out of bounds")
        return values[pointer.index]
    if pointer.kind == "buffer":
        raise RuntimeTinyOneError("Use read8/read16/read32 for buffer pointers")
    if pointer.kind == "field":
        obj = context.heap.get_address(pointer.address, pointer.generation)
        if obj.kind != "struct":
            raise RuntimeTinyOneError("Field pointer no longer points at a struct")
        fields = obj.value
        if not isinstance(fields, dict):
            raise RuntimeTinyOneError("Corrupt struct object")
        if pointer.field not in fields:
            raise RuntimeTinyOneError(f"Unknown field {pointer.field!r} on struct {obj.type_name!r}")
        return fields[pointer.field]
    raise RuntimeTinyOneError(f"Unknown raw pointer kind {pointer.kind!r}")


def runtime_pointer_store(context: TinyRuntimeContext, pointer: Value, value: Value) -> Value:
    pointer = runtime_expect_pointer(pointer, "ptr_store")
    if runtime_is_null(pointer):
        raise RuntimeTinyOneError("Cannot store through null")
    if pointer.kind == "object":
        obj = context.heap.get_address(pointer.address, pointer.generation)
        if obj.kind != "cell":
            raise RuntimeTinyOneError(
                "Object raw pointers can only store through pointer cells; "
                "use array or field pointers for aggregates"
            )
        obj.value = value
        return value
    if pointer.kind == "array":
        obj = context.heap.get_address(pointer.address, pointer.generation)
        if obj.kind != "array":
            raise RuntimeTinyOneError("Array pointer no longer points at an array")
        values = obj.value
        if not isinstance(values, list):
            raise RuntimeTinyOneError("Corrupt array object")
        if pointer.index < 0 or pointer.index >= len(values):
            raise RuntimeTinyOneError(f"Array pointer index {pointer.index} out of bounds")
        values[pointer.index] = value
        return value
    if pointer.kind == "buffer":
        raise RuntimeTinyOneError("Use write8/write16/write32 for buffer pointers")
    if pointer.kind == "field":
        obj = context.heap.get_address(pointer.address, pointer.generation)
        if obj.kind != "struct":
            raise RuntimeTinyOneError("Field pointer no longer points at a struct")
        fields = obj.value
        if not isinstance(fields, dict):
            raise RuntimeTinyOneError("Corrupt struct object")
        if pointer.field not in fields:
            raise RuntimeTinyOneError(f"Unknown field {pointer.field!r} on struct {obj.type_name!r}")
        fields[pointer.field] = value
        return value
    raise RuntimeTinyOneError(f"Unknown raw pointer kind {pointer.kind!r}")


def runtime_pointer_type(context: TinyRuntimeContext, pointer: Value) -> HeapRef:
    pointer = runtime_expect_pointer(pointer, "ptr_type")
    runtime_validate_pointer_base(context, pointer, "ptr_type")
    return context.heap.alloc_string(pointer.cast or pointer.kind)


def runtime_pointer_base(context: TinyRuntimeContext, pointer: Value) -> int:
    pointer = runtime_expect_pointer(pointer, "ptr_base")
    runtime_validate_pointer_base(context, pointer, "ptr_base")
    return pointer.address


def runtime_pointer_offset(context: TinyRuntimeContext, pointer: Value) -> int:
    pointer = runtime_expect_pointer(pointer, "ptr_offset")
    runtime_validate_pointer_base(context, pointer, "ptr_offset")
    if pointer.kind in ("array", "buffer"):
        return pointer.index
    return 0


def runtime_pointer_kind(context: TinyRuntimeContext, pointer: Value) -> HeapRef:
    pointer = runtime_expect_pointer(pointer, "ptr_kind")
    runtime_validate_pointer_base(context, pointer, "ptr_kind")
    return context.heap.alloc_string(pointer.kind)


def runtime_pointer_field(context: TinyRuntimeContext, pointer: Value) -> HeapRef:
    pointer = runtime_expect_pointer(pointer, "ptr_field")
    runtime_validate_pointer_base(context, pointer, "ptr_field")
    return context.heap.alloc_string(pointer.field if pointer.kind == "field" else "")


def runtime_pointer_eq(context: TinyRuntimeContext, lhs: Value, rhs: Value) -> int:
    lhs_pointer = runtime_expect_pointer(lhs, "ptr_eq")
    rhs_pointer = runtime_expect_pointer(rhs, "ptr_eq")
    runtime_validate_pointer_base(context, lhs_pointer, "ptr_eq")
    runtime_validate_pointer_base(context, rhs_pointer, "ptr_eq")
    return (
        1
        if runtime_pointer_identity(lhs_pointer) == runtime_pointer_identity(rhs_pointer)
        else 0
    )


def runtime_pointer_ne(context: TinyRuntimeContext, lhs: Value, rhs: Value) -> int:
    return 0 if runtime_pointer_eq(context, lhs, rhs) else 1


def runtime_cast_pointer(context: TinyRuntimeContext, pointer: Value, type_value: Value) -> RawPointer:
    pointer = runtime_expect_pointer(pointer, "cast_ptr")
    runtime_validate_pointer_base(context, pointer, "cast_ptr")
    type_name = runtime_expect_string(context, type_value, "cast_ptr")
    if type_name not in ("u8", "u16", "u32", "i8", "i16", "i32"):
        raise RuntimeTinyOneError(f"Unsupported pointer cast {type_name!r}")
    return RawPointer(
        pointer.address,
        pointer.kind,
        pointer.index,
        pointer.field,
        pointer.generation,
        type_name,
    )


def runtime_make_buffer(context: TinyRuntimeContext, size: Value) -> HeapRef:
    size_int = runtime_expect_int(size, "buffer")
    if size_int < 0:
        raise RuntimeTinyOneError("buffer() size must be non-negative")
    return context.heap.alloc_buffer(size_int)


def runtime_expect_buffer_pointer(
    context: TinyRuntimeContext, pointer: Value, operation: str
) -> tuple[bytearray, int]:
    pointer = runtime_expect_pointer(pointer, operation)
    if runtime_is_null(pointer):
        raise RuntimeTinyOneError(f"{operation} cannot use null")
    if pointer.kind != "buffer":
        raise RuntimeTinyOneError(f"{operation} expects a buffer pointer")
    obj = context.heap.get_address(pointer.address, pointer.generation)
    if obj.kind != "buffer":
        raise RuntimeTinyOneError("Buffer pointer no longer points at a buffer")
    data = obj.value
    if not isinstance(data, bytearray):
        raise RuntimeTinyOneError("Corrupt buffer object")
    return data, pointer.index


def runtime_read_uint(context: TinyRuntimeContext, pointer: Value, width: int, operation: str) -> int:
    data, offset = runtime_expect_buffer_pointer(context, pointer, operation)
    if offset < 0 or offset + width > len(data):
        raise RuntimeTinyOneError(f"{operation} out of bounds at byte offset {offset}")
    return int.from_bytes(data[offset : offset + width], "little", signed=False)


def runtime_write_uint(
    context: TinyRuntimeContext, pointer: Value, value: Value, width: int, operation: str
) -> int:
    data, offset = runtime_expect_buffer_pointer(context, pointer, operation)
    value_int = runtime_expect_int(value, operation)
    max_value = (1 << (width * 8)) - 1
    if value_int < 0 or value_int > max_value:
        raise RuntimeTinyOneError(f"{operation} value must be in range 0..{max_value}")
    if offset < 0 or offset + width > len(data):
        raise RuntimeTinyOneError(f"{operation} out of bounds at byte offset {offset}")
    data[offset : offset + width] = value_int.to_bytes(width, "little", signed=False)
    return value_int

# Per-builtin handlers.  Signature: (context, args) -> Value.
# Defined at module level so _BUILTIN_DISPATCH is a plain Final dict with no
# per-call closure allocation.

def _b_len(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind in ("array", "string", "buffer"):
        return len(obj.value) if isinstance(obj.value, (list, str, bytearray)) else 0
    if obj.kind == "struct":
        return len(obj.value) if isinstance(obj.value, dict) else 0
    if obj.kind == "map":
        return len(obj.value) if isinstance(obj.value, list) else 0
    raise RuntimeTinyOneError(f"len() does not support {obj.kind}")


def _b_array(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    count = runtime_expect_int(args[0], "array")
    if count < 0:
        raise RuntimeTinyOneError("array() length must be non-negative")
    return ctx.heap.alloc_array([args[1]] * count)


def _b_alloc(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_cell(args[0])


def _b_load(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "cell":
        raise RuntimeTinyOneError("load() expects a pointer cell")
    return obj.value if isinstance(obj.value, (int, HeapRef, RawPointer)) else 0


def _b_store(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "cell":
        raise RuntimeTinyOneError("store() expects a pointer cell")
    obj.value = args[1]
    return args[1]


def _b_free(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    ctx.heap.free(args[0])
    return 0


def _b_read(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    raw = ctx.read_raw()
    return int(raw) if _looks_like_int(raw) else ctx.heap.alloc_string(raw)


def _b_read_int(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    raw = ctx.read_raw()
    if not _looks_like_int(raw):
        raise RuntimeTinyOneError(f"read_int() expected integer input, got {raw!r}")
    return int(raw)


def _b_read_str(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_string(ctx.read_raw())


def _b_to_int(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    value = args[0]
    if isinstance(value, int):
        return value
    obj = ctx.heap.get(value)
    if obj.kind != "string" or not isinstance(obj.value, str) or not _looks_like_int(obj.value):
        raise RuntimeTinyOneError("to_int() expects an integer or numeric string")
    return int(obj.value)


def _b_ptr(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_make_pointer(ctx, args)


def _b_fieldptr(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_make_field_pointer(ctx, args[0], args[1])


def _b_ptr_addr(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_address(ctx, args[0])


def _b_ptr_at(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_at(ctx, args[0])


def _b_ptr_add(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_add(ctx, args[0], args[1])


def _b_ptr_load(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_load(ctx, args[0])


def _b_ptr_store(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_store(ctx, args[0], args[1])


def _b_ptr_type(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_type(ctx, args[0])


def _b_buffer(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_make_buffer(ctx, args[0])


def _b_is_null(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    pointer = runtime_expect_pointer(args[0], "is_null")
    runtime_validate_pointer_base(ctx, pointer, "is_null")
    return 1 if runtime_is_null(pointer) else 0


def _b_ptr_eq(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_eq(ctx, args[0], args[1])


def _b_ptr_ne(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_ne(ctx, args[0], args[1])


def _b_ptr_base(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_base(ctx, args[0])


def _b_ptr_offset(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_offset(ctx, args[0])


def _b_ptr_kind(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_kind(ctx, args[0])


def _b_ptr_field(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_pointer_field(ctx, args[0])


def _b_read8(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_read_uint(ctx, args[0], 1, "read8")


def _b_write8(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_write_uint(ctx, args[0], args[1], 1, "write8")


def _b_read16(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_read_uint(ctx, args[0], 2, "read16")


def _b_write16(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_write_uint(ctx, args[0], args[1], 2, "write16")


def _b_read32(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_read_uint(ctx, args[0], 4, "read32")


def _b_write32(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_write_uint(ctx, args[0], args[1], 4, "write32")


def _b_cast_ptr(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return runtime_cast_pointer(ctx, args[0], args[1])


# ---------------------------------------------------------------------------
# push/pop bring Python parity with the Rust BUILTINS table indices 33/34.
# Without these, Python verifies and runs Rust-generated artifacts that use
# dynamic array growth incorrectly. See `BUILTINS` in Rust/src/builtins.rs.
# ---------------------------------------------------------------------------


def _b_push(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "array":
        raise RuntimeTinyOneError(
            f"push() expects an array, got {obj.kind}"
        )
    values = obj.value
    if not isinstance(values, list):
        raise RuntimeTinyOneError("Corrupt array object")
    if len(values) >= 65_536:
        raise RuntimeTinyOneError("push() exceeds maximum length 65536")
    values.append(args[1])
    return len(values)


def _b_pop(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "array":
        raise RuntimeTinyOneError(
            f"pop() expects an array, got {obj.kind}"
        )
    values = obj.value
    if not isinstance(values, list):
        raise RuntimeTinyOneError("Corrupt array object")
    if not values:
        raise RuntimeTinyOneError("pop() cannot pop from an empty array")
    return values.pop()


# ---------------------------------------------------------------------------
# TinyOne typing system (mirror of Rust/src/runtime/typing.rs).
#
# Type IDs MUST match the Rust ordinals; the parity tests confirm that.
# ---------------------------------------------------------------------------


class TypeKind:
    UNIT = "unit"
    BOOL = "bool"
    I8 = "i8"
    I16 = "i16"
    I32 = "i32"
    I64 = "i64"
    U8 = "u8"
    U16 = "u16"
    U32 = "u32"
    U64 = "u64"
    BF16 = "bf16"
    FP16 = "fp16"
    FP32 = "fp32"
    FP64 = "fp64"
    CHAR = "Char"
    STRING = "String"
    CHAR_BUFFER = "CharBuffer"
    ARRAY = "Array"
    VEC = "Vec"
    BUFFER = "Buffer"
    MAP = "Map"
    DICTIONARY = "Dictionary"
    STRUCT = "Struct"
    RECORD = "Record"
    POINTER = "Pointer"
    REFERENCE = "Reference"
    BOX = "Box"
    ALLOC = "Alloc"
    FUNCTION = "Function"
    CLOSURE = "Closure"
    SUM = "Sum"
    ENUM = "Enum"
    TAGGED_UNION = "TaggedUnion"
    PHANTOM = "Phantom"
    ZST = "Zst"
    UNSAFE = "Unsafe"
    DYN = "Dyn"
    NULL = "Null"
    RESULT = "Result"
    OPTION = "Option"
    FILE_DESCRIPTOR = "FileDescriptor"
    MUTEX = "Mutex"
    ATOMIC = "Atomic"


_TYPE_KIND_IDS: Final[dict[str, int]] = {
    "unit": 0,
    "bool": 1,
    "i8": 2,
    "i16": 3,
    "i32": 4,
    "i64": 5,
    "u8": 6,
    "u16": 7,
    "u32": 8,
    "u64": 9,
    "bf16": 10,
    "fp16": 11,
    "fp32": 12,
    "fp64": 13,
    "Char": 14,
    "String": 15,
    "CharBuffer": 16,
    "Array": 17,
    "Vec": 18,
    "Buffer": 19,
    "Map": 20,
    "Dictionary": 21,
    "Struct": 22,
    "Record": 23,
    "Pointer": 24,
    "Reference": 25,
    "Box": 26,
    "Alloc": 27,
    "Function": 28,
    "Closure": 29,
    "Sum": 30,
    "Enum": 31,
    "TaggedUnion": 32,
    "Phantom": 33,
    "Zst": 34,
    "Unsafe": 35,
    "Dyn": 36,
    "Null": 37,
    "Result": 38,
    "Option": 39,
    "FileDescriptor": 40,
    "Mutex": 41,
    "Atomic": 42,
}

_INT_RANGES: Final[dict[str, tuple[int, int]]] = {
    "i8": (-(1 << 7), (1 << 7) - 1),
    "i16": (-(1 << 15), (1 << 15) - 1),
    "i32": (-(1 << 31), (1 << 31) - 1),
    "i64": (-(1 << 63), (1 << 63) - 1),
    "u8": (0, (1 << 8) - 1),
    "u16": (0, (1 << 16) - 1),
    "u32": (0, (1 << 32) - 1),
    "u64": (0, (1 << 64) - 1),
}

_INT_BITS: Final[dict[str, int]] = {
    "i8": 8, "i16": 16, "i32": 32, "i64": 64,
    "u8": 8, "u16": 16, "u32": 32, "u64": 64,
}


def _is_signed(name: str) -> bool:
    return name in ("i8", "i16", "i32", "i64")


def _is_unsigned(name: str) -> bool:
    return name in ("u8", "u16", "u32", "u64")


def _is_integer(name: str) -> bool:
    return _is_signed(name) or _is_unsigned(name)


def typing_smallest_fit(value: int) -> str:
    """Mirror of Rust `smallest_fit_literal`."""
    if value >= 0:
        if value <= 0xFF:
            return "u8"
        if value <= 0xFFFF:
            return "u16"
        if value <= 0xFFFF_FFFF:
            return "u32"
        return "u64"
    if -(1 << 7) <= value <= (1 << 7) - 1:
        return "i8"
    if -(1 << 15) <= value <= (1 << 15) - 1:
        return "i16"
    if -(1 << 31) <= value <= (1 << 31) - 1:
        return "i32"
    return "i64"


def typing_promote(lhs: str, rhs: str) -> str:
    """Mirror of Rust `promote_integer` — spec rule:
       target_bits = min(64, max(lhs_bits, rhs_bits) * 2)
       target signed if either operand signed."""
    if not _is_integer(lhs) or not _is_integer(rhs):
        raise RuntimeTinyOneError("integer promotion requires integer operands")
    if lhs == rhs:
        return lhs
    target_signed = _is_signed(lhs) or _is_signed(rhs)
    target_bits = min(64, max(_INT_BITS[lhs], _INT_BITS[rhs]) * 2)
    table = {
        (False, 8): "u8", (False, 16): "u16", (False, 32): "u32", (False, 64): "u64",
        (True, 8): "i8", (True, 16): "i16", (True, 32): "i32", (True, 64): "i64",
    }
    # round target_bits up to nearest defined width
    for bits in (8, 16, 32, 64):
        if target_bits <= bits:
            return table[(target_signed, bits)]
    return "u64" if not target_signed else "i64"


def typing_check_range(kind: str, value: int) -> int:
    if kind not in _INT_RANGES:
        raise RuntimeTinyOneError(f"{kind!r} is not an integer type with a defined range")
    lo, hi = _INT_RANGES[kind]
    if value < lo or value > hi:
        raise RuntimeTinyOneError(
            f"Runtime.Memory_Overflow: {value} out of range for {kind}"
        )
    return value


# ---------------------------------------------------------------------------
# Stdlib bridges (mirror of Rust/src/runtime/stdlib.rs).
# ---------------------------------------------------------------------------


def _b_vec_new(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_array([])


def _b_vec_clear(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "array" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("vec_clear expects a vec/array")
    obj.value.clear()
    return 0


def _b_map_new(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_map([])


def _map_key_equal(ctx: TinyRuntimeContext, lhs: Value, rhs: Value) -> bool:
    if isinstance(lhs, int) and isinstance(rhs, int):
        return lhs == rhs
    if isinstance(lhs, RawPointer) and isinstance(rhs, RawPointer):
        return (
            lhs.kind == rhs.kind
            and lhs.address == rhs.address
            and lhs.index == rhs.index
            and lhs.field == rhs.field
        )
    if isinstance(lhs, HeapRef) and isinstance(rhs, HeapRef):
        lobj = ctx.heap.get(lhs)
        robj = ctx.heap.get(rhs)
        if lobj.kind == "string" and robj.kind == "string":
            return lobj.value == robj.value
        return lhs.address == rhs.address and lhs.generation == rhs.generation
    return False


def _b_map_set(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "map" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("map_set expects a map")
    entries = obj.value
    key, value = args[1], args[2]
    for i, (k, _) in enumerate(entries):
        if _map_key_equal(ctx, k, key):
            entries[i] = (k, value)
            return value
    entries.append((key, value))
    return value


def _b_map_get(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "map" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("map_get expects a map")
    for k, v in obj.value:
        if _map_key_equal(ctx, k, args[1]):
            return v
    raise RuntimeTinyOneError("map_get: missing key")


def _b_map_has(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "map" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("map_has expects a map")
    return 1 if any(_map_key_equal(ctx, k, args[1]) for k, _ in obj.value) else 0


def _b_map_del(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "map" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("map_del expects a map")
    before = len(obj.value)
    obj.value[:] = [(k, v) for k, v in obj.value if not _map_key_equal(ctx, k, args[1])]
    return 1 if before != len(obj.value) else 0


def _b_map_len(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "map" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("map_len expects a map")
    return len(obj.value)


def _b_map_keys(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "map" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("map_keys expects a map")
    return ctx.heap.alloc_array([k for k, _ in obj.value])


def _b_map_values(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "map" or not isinstance(obj.value, list):
        raise RuntimeTinyOneError("map_values expects a map")
    return ctx.heap.alloc_array([v for _, v in obj.value])


# I/O fds: keep identical to Rust constants.

IO_FD_STDIN = 0
IO_FD_STDOUT = 1
IO_FD_STDERR = 2


def _b_io_stdout(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return IO_FD_STDOUT


def _b_io_stderr(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return IO_FD_STDERR


def _b_io_stdin(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return IO_FD_STDIN


def _b_io_write(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    fd = runtime_expect_int(args[0], "io_write")
    text = runtime_expect_string(ctx, args[1], "io_write")
    encoded = text.encode("utf-8")
    if fd == IO_FD_STDOUT:
        ctx.io_stdout.append(text)
    elif fd == IO_FD_STDERR:
        ctx.io_stderr.append(text)
    elif fd == IO_FD_STDIN:
        raise RuntimeTinyOneError("io_write: cannot write to stdin (fd 0)")
    else:
        raise RuntimeTinyOneError(f"io_write: unsupported fd {fd}")
    return len(encoded)


def _b_io_writeln(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    fd = runtime_expect_int(args[0], "io_writeln")
    text = runtime_expect_string(ctx, args[1], "io_writeln")
    encoded = text.encode("utf-8")
    if fd == IO_FD_STDOUT:
        ctx.io_stdout.append(text)
        ctx.io_stdout.append("\n")
    elif fd == IO_FD_STDERR:
        ctx.io_stderr.append(text)
        ctx.io_stderr.append("\n")
    elif fd == IO_FD_STDIN:
        raise RuntimeTinyOneError("io_writeln: cannot write to stdin (fd 0)")
    else:
        raise RuntimeTinyOneError(f"io_writeln: unsupported fd {fd}")
    return len(encoded) + 1


def _b_io_read_line(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_string(ctx.read_raw())


def _b_io_flush(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return 0


def _b_io_capture_stdout(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    text = "".join(ctx.io_stdout)
    ctx.io_stdout.clear()
    return ctx.heap.alloc_string(text)


def _b_io_capture_stderr(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    text = "".join(ctx.io_stderr)
    ctx.io_stderr.clear()
    return ctx.heap.alloc_string(text)


# String / Unicode helpers.

def _b_str_byte_len(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    text = runtime_expect_string(ctx, args[0], "str_byte_len")
    return len(text.encode("utf-8"))


def _b_str_char_len(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    text = runtime_expect_string(ctx, args[0], "str_char_len")
    return len(text)


def _b_str_byte_at(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    text = runtime_expect_string(ctx, args[0], "str_byte_at")
    index = runtime_expect_int(args[1], "str_byte_at")
    if index < 0:
        raise RuntimeTinyOneError("str_byte_at: negative index")
    data = text.encode("utf-8")
    if index >= len(data):
        raise RuntimeTinyOneError("str_byte_at: index out of bounds")
    return data[index]


def _b_str_char_at(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    text = runtime_expect_string(ctx, args[0], "str_char_at")
    index = runtime_expect_int(args[1], "str_char_at")
    if index < 0:
        raise RuntimeTinyOneError("str_char_at: negative index")
    if index >= len(text):
        raise RuntimeTinyOneError("str_char_at: index out of bounds")
    return ctx.heap.alloc_string(text[index])


def _b_str_slice(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    text = runtime_expect_string(ctx, args[0], "str_slice")
    start = runtime_expect_int(args[1], "str_slice")
    end = runtime_expect_int(args[2], "str_slice")
    if start < 0 or end < 0:
        raise RuntimeTinyOneError("str_slice: negative bound")
    if end < start:
        raise RuntimeTinyOneError("str_slice: end < start")
    if start > len(text) or end > len(text):
        raise RuntimeTinyOneError("str_slice: bound out of range")
    return ctx.heap.alloc_string(text[start:end])


def _b_str_concat(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    left = runtime_expect_string(ctx, args[0], "str_concat")
    right = runtime_expect_string(ctx, args[1], "str_concat")
    return ctx.heap.alloc_string(left + right)


def _b_str_is_utf8(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind == "string":
        return 1
    if obj.kind == "buffer":
        try:
            bytes(obj.value).decode("utf-8")
            return 1
        except UnicodeDecodeError:
            return 0
    raise RuntimeTinyOneError("str_is_utf8 expects a String or Buffer")


def _b_str_from_buffer(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "buffer":
        raise RuntimeTinyOneError("str_from_buffer expects a Buffer")
    try:
        text = bytes(obj.value).decode("utf-8")
    except UnicodeDecodeError as error:
        raise RuntimeTinyOneError("str_from_buffer: invalid UTF-8") from error
    return ctx.heap.alloc_string(text)


# Threading / Sync.

def _b_mutex_new(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_struct("tinyone.sync.Mutex", {"locked": 0})


def _b_mutex_lock(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "struct" or not isinstance(obj.value, dict):
        raise RuntimeTinyOneError("mutex_lock expects a Mutex")
    state = obj.value.get("locked")
    if not isinstance(state, int):
        raise RuntimeTinyOneError("mutex_lock: corrupt mutex state")
    if state != 0:
        raise RuntimeTinyOneError("mutex_lock: already locked (deadlock)")
    obj.value["locked"] = 1
    return 1


def _b_mutex_unlock(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "struct" or not isinstance(obj.value, dict):
        raise RuntimeTinyOneError("mutex_unlock expects a Mutex")
    state = obj.value.get("locked")
    if not isinstance(state, int):
        raise RuntimeTinyOneError("mutex_unlock: corrupt mutex state")
    if state == 0:
        raise RuntimeTinyOneError("mutex_unlock: not locked")
    obj.value["locked"] = 0
    return 0


def _b_atomic_new(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    init = runtime_expect_int(args[0], "atomic_new")
    return ctx.heap.alloc_struct("tinyone.sync.Atomic", {"value": init})


def _b_atomic_load(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    obj = ctx.heap.get(args[0])
    if obj.kind != "struct" or not isinstance(obj.value, dict):
        raise RuntimeTinyOneError("atomic_load expects an Atomic")
    return obj.value["value"]


def _b_atomic_store(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    new_value = runtime_expect_int(args[1], "atomic_store")
    obj = ctx.heap.get(args[0])
    if obj.kind != "struct" or not isinstance(obj.value, dict):
        raise RuntimeTinyOneError("atomic_store expects an Atomic")
    obj.value["value"] = new_value
    return new_value


def _b_atomic_add(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    delta = runtime_expect_int(args[1], "atomic_add")
    obj = ctx.heap.get(args[0])
    if obj.kind != "struct" or not isinstance(obj.value, dict):
        raise RuntimeTinyOneError("atomic_add expects an Atomic")
    current = obj.value.get("value")
    if not isinstance(current, int):
        raise RuntimeTinyOneError("atomic_add: corrupt atomic state")
    nxt = current + delta
    if nxt < -(1 << 63) or nxt > (1 << 63) - 1:
        raise RuntimeTinyOneError("Runtime.Memory_Overflow: atomic_add overflow")
    obj.value["value"] = nxt
    return nxt


# Result / Option.

_VARIANT_OK = 1
_VARIANT_ERR = 0
_VARIANT_SOME = 1
_VARIANT_NONE = 0


def _b_result_ok(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_struct(
        "tinyone.result.Result", {"tag": _VARIANT_OK, "payload": args[0]}
    )


def _b_result_err(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_struct(
        "tinyone.result.Result", {"tag": _VARIANT_ERR, "payload": args[0]}
    )


def _variant_tag(ctx: TinyRuntimeContext, target: Value, type_name: str, op: str) -> int:
    obj = ctx.heap.get(target)
    if obj.type_name != type_name:
        raise RuntimeTinyOneError(
            f"{op}: expected {type_name}, got {obj.type_name!r}"
        )
    if obj.kind != "struct" or not isinstance(obj.value, dict):
        raise RuntimeTinyOneError(f"{op}: corrupt {type_name}")
    tag = obj.value.get("tag")
    if not isinstance(tag, int):
        raise RuntimeTinyOneError(f"{op}: tag must be an integer")
    return tag


def _variant_payload(ctx: TinyRuntimeContext, target: Value, type_name: str, op: str) -> Value:
    obj = ctx.heap.get(target)
    if obj.type_name != type_name:
        raise RuntimeTinyOneError(
            f"{op}: expected {type_name}, got {obj.type_name!r}"
        )
    if obj.kind != "struct" or not isinstance(obj.value, dict):
        raise RuntimeTinyOneError(f"{op}: corrupt {type_name}")
    return obj.value.get("payload", 0)


def _b_result_is_ok(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return 1 if _variant_tag(ctx, args[0], "tinyone.result.Result", "result_is_ok") == _VARIANT_OK else 0


def _b_result_is_err(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return 1 if _variant_tag(ctx, args[0], "tinyone.result.Result", "result_is_err") == _VARIANT_ERR else 0


def _b_result_unwrap(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    if _variant_tag(ctx, args[0], "tinyone.result.Result", "result_unwrap") != _VARIANT_OK:
        raise RuntimeTinyOneError("result_unwrap: called on Err")
    return _variant_payload(ctx, args[0], "tinyone.result.Result", "result_unwrap")


def _b_result_unwrap_err(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    if _variant_tag(ctx, args[0], "tinyone.result.Result", "result_unwrap_err") != _VARIANT_ERR:
        raise RuntimeTinyOneError("result_unwrap_err: called on Ok")
    return _variant_payload(ctx, args[0], "tinyone.result.Result", "result_unwrap_err")


def _b_option_some(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_struct(
        "tinyone.option.Option", {"tag": _VARIANT_SOME, "payload": args[0]}
    )


def _b_option_none(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return ctx.heap.alloc_struct(
        "tinyone.option.Option", {"tag": _VARIANT_NONE, "payload": 0}
    )


def _b_option_is_some(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return 1 if _variant_tag(ctx, args[0], "tinyone.option.Option", "option_is_some") == _VARIANT_SOME else 0


def _b_option_is_none(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return 1 if _variant_tag(ctx, args[0], "tinyone.option.Option", "option_is_none") == _VARIANT_NONE else 0


def _b_option_unwrap(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    if _variant_tag(ctx, args[0], "tinyone.option.Option", "option_unwrap") != _VARIANT_SOME:
        raise RuntimeTinyOneError("option_unwrap: called on None")
    return _variant_payload(ctx, args[0], "tinyone.option.Option", "option_unwrap")


# System introspection.

def _b_sys_argc(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return len(ctx.sys_args)


def _b_sys_argv(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    index = runtime_expect_int(args[0], "sys_argv")
    if index < 0 or index >= len(ctx.sys_args):
        raise RuntimeTinyOneError("sys_argv: index out of range")
    return ctx.heap.alloc_string(ctx.sys_args[index])


def _b_sys_env_has(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    key = runtime_expect_string(ctx, args[0], "sys_env_has")
    return 1 if key in ctx.sys_env else 0


def _b_sys_env_get(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    key = runtime_expect_string(ctx, args[0], "sys_env_get")
    if key not in ctx.sys_env:
        raise RuntimeTinyOneError(f"sys_env_get: missing variable {key!r}")
    return ctx.heap.alloc_string(ctx.sys_env[key])


# Path / FS.

def _b_path_join(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    left = runtime_expect_string(ctx, args[0], "path_join")
    right = runtime_expect_string(ctx, args[1], "path_join")
    if right.startswith("/"):
        joined = right
    elif not left:
        joined = right
    elif left.endswith("/"):
        joined = left + right
    else:
        joined = f"{left}/{right}"
    return ctx.heap.alloc_string(joined)


def _b_path_basename(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    path = runtime_expect_string(ctx, args[0], "path_basename")
    return ctx.heap.alloc_string(Path(path).name)


def _b_path_dirname(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    path = runtime_expect_string(ctx, args[0], "path_dirname")
    parent = Path(path).parent
    rendered = str(parent) if str(parent) != "." or path.startswith(("/", ".")) else ""
    if rendered == "." and not path.startswith("."):
        rendered = ""
    return ctx.heap.alloc_string(rendered if rendered != "." else "")


def _b_fs_read(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    path = runtime_expect_string(ctx, args[0], "fs_read")
    try:
        data = Path(path).read_bytes()
    except OSError as error:
        raise RuntimeTinyOneError(f"fs_read: {error}") from error
    return ctx.heap.alloc(HeapObject("buffer", bytearray(data)))


def _b_fs_write(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    path = runtime_expect_string(ctx, args[0], "fs_write")
    obj = ctx.heap.get(args[1])
    if obj.kind != "buffer" or not isinstance(obj.value, bytearray):
        raise RuntimeTinyOneError("fs_write expects a buffer payload")
    try:
        Path(path).write_bytes(bytes(obj.value))
    except OSError as error:
        raise RuntimeTinyOneError(f"fs_write: {error}") from error
    return len(obj.value)


def _b_fs_exists(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    path = runtime_expect_string(ctx, args[0], "fs_exists")
    return 1 if Path(path).exists() else 0


def _b_fs_list_dir(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    path = runtime_expect_string(ctx, args[0], "fs_list_dir")
    try:
        entries = sorted(p.name for p in Path(path).iterdir())
    except OSError as error:
        raise RuntimeTinyOneError(f"fs_list_dir: {error}") from error
    return ctx.heap.alloc_array([ctx.heap.alloc_string(name) for name in entries])


# Math / Logic.

_MATH_CONSTANTS: Final[dict[str, int]] = {
    "PI_THOUSANDTHS": 3142,
    "E_THOUSANDTHS": 2718,
    "TAU_THOUSANDTHS": 6283,
    "MAX_I64": (1 << 63) - 1,
    "MIN_I64": -(1 << 63),
}


def _b_math_const(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    key = runtime_expect_string(ctx, args[0], "math_const")
    if key not in _MATH_CONSTANTS:
        raise RuntimeTinyOneError(f"math_const: unknown constant {key!r}")
    return _MATH_CONSTANTS[key]


def _b_math_abs(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    value = runtime_expect_int(args[0], "math_abs")
    result = -value if value < 0 else value
    if result < -(1 << 63) or result > (1 << 63) - 1:
        raise RuntimeTinyOneError("Runtime.Memory_Overflow: math_abs")
    return result


def _b_math_min(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return min(
        runtime_expect_int(args[0], "math_min"),
        runtime_expect_int(args[1], "math_min"),
    )


def _b_math_max(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return max(
        runtime_expect_int(args[0], "math_max"),
        runtime_expect_int(args[1], "math_max"),
    )


def _b_logic_and(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    a = runtime_expect_int(args[0], "logic_and")
    b = runtime_expect_int(args[1], "logic_and")
    return 1 if a != 0 and b != 0 else 0


def _b_logic_or(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    a = runtime_expect_int(args[0], "logic_or")
    b = runtime_expect_int(args[1], "logic_or")
    return 1 if a != 0 or b != 0 else 0


def _b_logic_not(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return 1 if runtime_expect_int(args[0], "logic_not") == 0 else 0


def _b_logic_xor(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    a = runtime_expect_int(args[0], "logic_xor") != 0
    b = runtime_expect_int(args[1], "logic_xor") != 0
    return 1 if a != b else 0


# Typing-system builtins.

def _b_type_of(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    value = args[0]
    if isinstance(value, int):
        return ctx.heap.alloc_string(TypeKind.I64)
    if isinstance(value, RawPointer):
        if runtime_is_null(value):
            return ctx.heap.alloc_string(TypeKind.NULL)
        return ctx.heap.alloc_string(TypeKind.POINTER)
    if isinstance(value, HeapRef):
        obj = ctx.heap.get(value)
        if obj.kind == "string":
            return ctx.heap.alloc_string(TypeKind.STRING)
        if obj.kind == "array":
            return ctx.heap.alloc_string(TypeKind.VEC)
        if obj.kind == "buffer":
            return ctx.heap.alloc_string(TypeKind.BUFFER)
        if obj.kind == "map":
            return ctx.heap.alloc_string(TypeKind.MAP)
        if obj.kind == "cell":
            return ctx.heap.alloc_string(TypeKind.ALLOC)
        if obj.kind == "struct":
            if obj.type_name == "tinyone.result.Result":
                return ctx.heap.alloc_string(TypeKind.RESULT)
            if obj.type_name == "tinyone.option.Option":
                return ctx.heap.alloc_string(TypeKind.OPTION)
            if obj.type_name == "tinyone.sync.Mutex":
                return ctx.heap.alloc_string(TypeKind.MUTEX)
            if obj.type_name == "tinyone.sync.Atomic":
                return ctx.heap.alloc_string(TypeKind.ATOMIC)
            return ctx.heap.alloc_string(TypeKind.STRUCT)
    raise RuntimeTinyOneError("type_of: unsupported value")


def _b_type_id(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    name = runtime_expect_string(ctx, args[0], "type_id")
    if name not in _TYPE_KIND_IDS:
        raise RuntimeTinyOneError(f"type_id: unknown type {name!r}")
    return _TYPE_KIND_IDS[name]


def _b_smallest_fit(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    value = runtime_expect_int(args[0], "smallest_fit")
    return ctx.heap.alloc_string(typing_smallest_fit(value))


def _b_promote(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    lhs = runtime_expect_string(ctx, args[0], "promote")
    rhs = runtime_expect_string(ctx, args[1], "promote")
    return ctx.heap.alloc_string(typing_promote(lhs, rhs))


def _b_check_int_range(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    value = runtime_expect_int(args[0], "check_int_range")
    name = runtime_expect_string(ctx, args[1], "check_int_range")
    return typing_check_range(name, value)


def _typed_arith(
    ctx: TinyRuntimeContext,
    args: list[Value],
    operation: str,
    op: Callable[[int, int], int],
) -> Value:
    lhs = runtime_expect_int(args[0], operation)
    rhs = runtime_expect_int(args[1], operation)
    name = runtime_expect_string(ctx, args[2], operation)
    if name not in _INT_RANGES:
        raise RuntimeTinyOneError(f"{operation}: {name!r} is not an integer type")
    if operation == "typed_div" and rhs == 0:
        raise RuntimeTinyOneError("Runtime.Division_By_Zero")
    result = op(lhs, rhs)
    return typing_check_range(name, result)


def _b_typed_add(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return _typed_arith(ctx, args, "typed_add", lambda a, b: a + b)


def _b_typed_sub(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return _typed_arith(ctx, args, "typed_sub", lambda a, b: a - b)


def _b_typed_mul(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return _typed_arith(ctx, args, "typed_mul", lambda a, b: a * b)


def _b_typed_div(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    return _typed_arith(ctx, args, "typed_div", lambda a, b: a // b)


def _b_typed_neg(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    value = runtime_expect_int(args[0], "typed_neg")
    name = runtime_expect_string(ctx, args[1], "typed_neg")
    if not _is_signed(name):
        raise RuntimeTinyOneError(f"typed_neg: {name} is not signed")
    return typing_check_range(name, -value)


def _b_assert(ctx: TinyRuntimeContext, args: list[Value]) -> Value:
    value = runtime_expect_int(args[0], "assert")
    if value == 0:
        if len(args) > 1:
            detail = runtime_expect_string(ctx, args[1], "assert")
        else:
            detail = "assertion failed"
        raise RuntimeTinyOneError(f"Assertion failed: {detail}")
    return 1


_BUILTIN_DISPATCH: Final[dict[str, Callable[[TinyRuntimeContext, list[Value]], Value]]] = {
    "len":        _b_len,
    "array":      _b_array,
    "alloc":      _b_alloc,
    "load":       _b_load,
    "store":      _b_store,
    "free":       _b_free,
    "read":       _b_read,
    "read_int":   _b_read_int,
    "read_str":   _b_read_str,
    "to_int":     _b_to_int,
    "ptr":        _b_ptr,
    "fieldptr":   _b_fieldptr,
    "ptr_addr":   _b_ptr_addr,
    "ptr_at":     _b_ptr_at,
    "ptr_add":    _b_ptr_add,
    "ptr_load":   _b_ptr_load,
    "ptr_store":  _b_ptr_store,
    "ptr_type":   _b_ptr_type,
    "buffer":     _b_buffer,
    "is_null":    _b_is_null,
    "ptr_eq":     _b_ptr_eq,
    "ptr_ne":     _b_ptr_ne,
    "ptr_base":   _b_ptr_base,
    "ptr_offset": _b_ptr_offset,
    "ptr_kind":   _b_ptr_kind,
    "ptr_field":  _b_ptr_field,
    "read8":      _b_read8,
    "write8":     _b_write8,
    "read16":     _b_read16,
    "write16":    _b_write16,
    "read32":     _b_read32,
    "write32":    _b_write32,
    "cast_ptr":   _b_cast_ptr,
    "push":       _b_push,
    "pop":        _b_pop,
    # Phase 2 stdlib bridge handlers (must be present for every BuiltinDef
    # entry added after slot 34).
    "vec_new":          _b_vec_new,
    "vec_clear":        _b_vec_clear,
    "map_new":          _b_map_new,
    "map_set":          _b_map_set,
    "map_get":          _b_map_get,
    "map_has":          _b_map_has,
    "map_del":          _b_map_del,
    "map_len":          _b_map_len,
    "map_keys":         _b_map_keys,
    "map_values":       _b_map_values,
    "io_stdout":        _b_io_stdout,
    "io_stderr":        _b_io_stderr,
    "io_stdin":         _b_io_stdin,
    "io_write":         _b_io_write,
    "io_writeln":       _b_io_writeln,
    "io_read_line":     _b_io_read_line,
    "io_flush":         _b_io_flush,
    "io_capture_stdout": _b_io_capture_stdout,
    "io_capture_stderr": _b_io_capture_stderr,
    "str_byte_len":     _b_str_byte_len,
    "str_char_len":     _b_str_char_len,
    "str_byte_at":      _b_str_byte_at,
    "str_char_at":      _b_str_char_at,
    "str_slice":        _b_str_slice,
    "str_concat":       _b_str_concat,
    "str_is_utf8":      _b_str_is_utf8,
    "str_from_buffer":  _b_str_from_buffer,
    "mutex_new":        _b_mutex_new,
    "mutex_lock":       _b_mutex_lock,
    "mutex_unlock":     _b_mutex_unlock,
    "atomic_new":       _b_atomic_new,
    "atomic_load":      _b_atomic_load,
    "atomic_store":     _b_atomic_store,
    "atomic_add":       _b_atomic_add,
    "result_ok":        _b_result_ok,
    "result_err":       _b_result_err,
    "result_is_ok":     _b_result_is_ok,
    "result_is_err":    _b_result_is_err,
    "result_unwrap":    _b_result_unwrap,
    "result_unwrap_err": _b_result_unwrap_err,
    "option_some":      _b_option_some,
    "option_none":      _b_option_none,
    "option_is_some":   _b_option_is_some,
    "option_is_none":   _b_option_is_none,
    "option_unwrap":    _b_option_unwrap,
    "sys_argc":         _b_sys_argc,
    "sys_argv":         _b_sys_argv,
    "sys_env_has":      _b_sys_env_has,
    "sys_env_get":      _b_sys_env_get,
    "path_join":        _b_path_join,
    "path_basename":    _b_path_basename,
    "path_dirname":     _b_path_dirname,
    "fs_read":          _b_fs_read,
    "fs_write":         _b_fs_write,
    "fs_exists":        _b_fs_exists,
    "fs_list_dir":      _b_fs_list_dir,
    "math_const":       _b_math_const,
    "math_abs":         _b_math_abs,
    "math_min":         _b_math_min,
    "math_max":         _b_math_max,
    "logic_and":        _b_logic_and,
    "logic_or":         _b_logic_or,
    "logic_not":        _b_logic_not,
    "logic_xor":        _b_logic_xor,
    "type_of":          _b_type_of,
    "type_id":          _b_type_id,
    "smallest_fit":     _b_smallest_fit,
    "promote":          _b_promote,
    "check_int_range": _b_check_int_range,
    "typed_add":        _b_typed_add,
    "typed_sub":        _b_typed_sub,
    "typed_mul":        _b_typed_mul,
    "typed_div":        _b_typed_div,
    "typed_neg":        _b_typed_neg,
    "assert":           _b_assert,
}

def runtime_call_builtin(
    context: TinyRuntimeContext, builtin_index: int, args: list[Value]
) -> Value:
    try:
        builtin = _BUILTINS[builtin_index]
    except IndexError as error:
        raise RuntimeTinyOneError(f"Invalid builtin index {builtin_index}") from error
    if not builtin.min_args <= len(args) <= builtin.max_args:
        raise RuntimeTinyOneError(
            f"Builtin {builtin.name!r} expects {builtin.min_args}..{builtin.max_args} "
            f"argument(s), got {len(args)}"
        )
    # _BUILTIN_DISPATCH is keyed by the same names as _BUILTINS; a missing key
    # is a programming error (builtin added to _BUILTINS without a handler).
    return _BUILTIN_DISPATCH[builtin.name](context, args)

def _looks_like_int(text: str) -> bool:
    if not text:
        return False
    if text[0] in "+-":
        return len(text) > 1 and text[1:].isdigit()
    return text.isdigit()


def runtime_format(context: TinyRuntimeContext, value: Value) -> str:
    return _runtime_format(context, value, set())


def _runtime_format(context: TinyRuntimeContext, value: Value, seen: set[int]) -> str:
    if isinstance(value, int):
        return str(value)
    if isinstance(value, RawPointer):
        suffix = f":{value.cast}" if value.cast else ""
        if runtime_is_null(value):
            return "null"
        if value.kind == "array":
            return f"ptr(array@{value.address}[{value.index}]{suffix})"
        if value.kind == "buffer":
            return f"ptr(buffer@{value.address}+{value.index}{suffix})"
        if value.kind == "field":
            return f"ptr(field@{value.address}.{value.field}{suffix})"
        return f"ptr({value.kind}@{value.address}{suffix})"
    obj = context.heap.get(value)
    if value.address in seen:
        return f"&{value.address}<cycle>"
    seen.add(value.address)
    try:
        if obj.kind == "string":
            return str(obj.value)
        if obj.kind == "array":
            values = obj.value
            if not isinstance(values, list):
                raise RuntimeTinyOneError("Corrupt array object")
            return "[" + ", ".join(_runtime_format(context, item, seen) for item in values) + "]"
        if obj.kind == "buffer":
            data = obj.value
            if not isinstance(data, bytearray):
                raise RuntimeTinyOneError("Corrupt buffer object")
            return "buffer[" + " ".join(f"{byte:02x}" for byte in data) + "]"
        if obj.kind == "struct":
            fields = obj.value
            if not isinstance(fields, dict):
                raise RuntimeTinyOneError("Corrupt struct object")
            rendered = ", ".join(
                f"{name}: {_runtime_format(context, field_value, seen)}"
                for name, field_value in fields.items()
            )
            return f"{obj.type_name}{{{rendered}}}"
        if obj.kind == "cell":
            inner = obj.value if isinstance(obj.value, (int, HeapRef, RawPointer)) else 0
            return f"&{value.address}({_runtime_format(context, inner, seen)})"
        if obj.kind == "map":
            entries = obj.value
            if not isinstance(entries, list):
                raise RuntimeTinyOneError("Corrupt map object")
            rendered = ", ".join(
                f"{_runtime_format(context, k, seen)}: {_runtime_format(context, v, seen)}"
                for k, v in entries
            )
            return f"map{{{rendered}}}"
        raise RuntimeTinyOneError(f"Cannot format heap object {obj.kind!r}")
    finally:
        seen.remove(value.address)


def runtime_print(context: TinyRuntimeContext, stdout: TextIO, value: Value) -> None:
    print(runtime_format(context, value), file=stdout)


class TinyMemory:
    """
    Zero-initialized stack-frame slot storage.

    Values are addressed by integer handles. Undefined control-flow paths read
    as 0, which keeps the runtime model simple and predictable.
    """

    __slots__ = ("_values",)

    def __init__(self, slot_count: int) -> None:
        if slot_count < 0:
            raise ValueError("slot_count must be non-negative")
        self._values = [0] * slot_count

    def reset(self) -> None:
        self._values[:] = [0] * len(self._values)

    def load(self, slot: int) -> Value:
        self._check_slot(slot)
        return self._values[slot]

    def store(self, slot: int, value: Value) -> None:
        self._check_slot(slot)
        self._values[slot] = value

    def snapshot(self) -> tuple[Value, ...]:
        return tuple(self._values)

    def _check_slot(self, slot: int) -> None:
        if slot < 0 or slot >= len(self._values):
            raise RuntimeTinyOneError(f"Invalid memory slot {slot}")

class VM:
    """Portable bytecode interpreter."""

    __slots__ = ("_program", "_memory", "_stdout", "_context")

    def __init__(
        self,
        program: Program,
        memory: TinyMemory,
        stdout: TextIO,
        context: TinyRuntimeContext | None = None,
    ) -> None:
        self._program = program
        self._memory = memory
        self._stdout = stdout
        self._context = TinyRuntimeContext() if context is None else context

    def run(self) -> None:
        self._run_chunk(self._program.code, self._memory, "main")

    def _run_chunk(
        self, code: tuple[Instr, ...], memory: TinyMemory, chunk_name: str
    ) -> Value | None:
        stack: list[Value] = []
        stdout = self._stdout
        context = self._context
        pc = 0

        while True:
            instr = code[pc]
            pc += 1
            op = instr.op
            arg = instr.arg
            arg2 = instr.arg2

            if op == Op.PUSH_INT:
                stack.append(arg)
            elif op == Op.PUSH_NULL:
                stack.append(runtime_null())
            elif op == Op.PUSH_STRING:
                stack.append(context.heap.alloc_string(self._program.strings[arg]))
            elif op == Op.LOAD:
                stack.append(memory.load(arg))
            elif op == Op.STORE:
                memory.store(arg, stack.pop())
            elif op == Op.ADD:
                rhs = stack.pop()
                stack[-1] = runtime_add(stack[-1], rhs)
            elif op == Op.SUB:
                rhs = stack.pop()
                stack[-1] = runtime_sub(stack[-1], rhs)
            elif op == Op.MUL:
                rhs = stack.pop()
                stack[-1] = runtime_mul(stack[-1], rhs)
            elif op == Op.DIV:
                rhs = stack.pop()
                stack[-1] = checked_div(stack[-1], rhs)
            elif op == Op.NEG:
                stack[-1] = runtime_neg(stack[-1])
            elif op in (Op.LT, Op.LTE, Op.GT, Op.GTE, Op.EQ, Op.NE):
                rhs = stack.pop()
                stack[-1] = runtime_compare(op, stack[-1], rhs)
            elif op == Op.JUMP:
                pc = arg
            elif op == Op.JUMP_IF_ZERO:
                if runtime_is_false(stack.pop()):
                    pc = arg
            elif op == Op.CALL:
                stack.append(self._call_function(arg, stack, arg2))
            elif op == Op.MAKE_ARRAY:
                values = [stack.pop() for _ in range(arg)]
                values.reverse()
                stack.append(runtime_make_array(context, values))
            elif op == Op.INDEX:
                index = stack.pop()
                container = stack.pop()
                stack.append(runtime_index(context, container, index))
            elif op == Op.SET_INDEX:
                value = stack.pop()
                index = stack.pop()
                container = stack.pop()
                runtime_set_index(context, container, index, value)
            elif op == Op.MAKE_STRUCT:
                values = [stack.pop() for _ in range(arg2)]
                values.reverse()
                struct = self._program.structs[arg]
                stack.append(runtime_make_struct(context, struct.name, struct.fields, values))
            elif op == Op.GET_FIELD:
                stack[-1] = runtime_get_field(context, stack[-1], self._program.fields[arg])
            elif op == Op.SET_FIELD:
                value = stack.pop()
                target = stack.pop()
                runtime_set_field(context, target, self._program.fields[arg], value)
            elif op == Op.BUILTIN:
                args = [stack.pop() for _ in range(arg2)]
                args.reverse()
                stack.append(runtime_call_builtin(context, arg, args))
            elif op == Op.RETURN:
                return stack.pop()
            elif op == Op.PRINT:
                runtime_print(context, stdout, stack.pop())
            elif op == Op.HALT:
                if stack:
                    raise RuntimeTinyOneError(
                        f"Internal stack imbalance at halt in {chunk_name}"
                    )
                return
            else:
                raise RuntimeTinyOneError(f"Unknown opcode {op!r}")

    def _call_function(
        self, function_index: int, caller_stack: list[Value], arg_count: int
    ) -> Value:
        function = self._program.functions[function_index]
        args = [caller_stack.pop() for _ in range(arg_count)]
        args.reverse()
        memory = TinyMemory(function.slot_count)
        for slot, value in enumerate(args):
            memory.store(slot, value)
        result = self._run_chunk(function.code, memory, function.name)
        if result is None:
            raise RuntimeTinyOneError(f"Function {function.name!r} returned no value")
        return result


class JitCache:
    """
    Compiles TinyOne bytecode into generated Python functions.

    Branch-free main programs use the original locals-based path: the stack is
    resolved at codegen time, each virtual stack slot maps to a named Python
    local (_s0, _s1, ...), and binary ops fold two locals into the lower slot in
    place. Programs with functions or loops use generated dispatch code so
    branch targets, function calls, and returns share the exact verified
    bytecode semantics used by the VM.

    Example: `let x = 1 + 2 * 3; print x`

    Unoptimized JIT (old):
        stack = []
        push = stack.append
        pop  = stack.pop
        push(1); push(2); push(3)
        _rhs = pop(); stack[-1] = stack[-1] * _rhs   # MUL
        _rhs = pop(); stack[-1] = stack[-1] + _rhs   # ADD
        store(0, pop()); push(load(0))
        write(str(pop()) + '\\n')

    Optimized JIT (new, after peephole folds 2*3=6):
        _s0 = 1
        _s1 = 6
        _s0 = _s0 + _s1
        memory.store(0, _s0)
        _s0 = memory.load(0)
        stdout.write(str(_s0) + '\\n')

    Precondition: program has been verified by BytecodeVerifier.
    """

    __slots__ = ("_cache",)

    def __init__(self) -> None:
        self._cache: dict[
            str, Callable[[TinyMemory, TextIO, TinyRuntimeContext | None], None]
        ] = {}

    def compile(
        self, program: Program
    ) -> Callable[[TinyMemory, TextIO, TinyRuntimeContext | None], None]:
        key = program.fingerprint
        cached = self._cache.get(key)
        if cached is not None:
            return cached

        source = self._build_source(program)
        namespace: dict[str, object] = {
            "checked_div": checked_div,
            "RuntimeTinyOneError": RuntimeTinyOneError,
            "TinyMemory": TinyMemory,
            "TinyRuntimeContext": TinyRuntimeContext,
            "runtime_add": runtime_add,
            "runtime_sub": runtime_sub,
            "runtime_mul": runtime_mul,
            "runtime_neg": runtime_neg,
            "runtime_compare": runtime_compare,
            "runtime_is_false": runtime_is_false,
            "runtime_make_array": runtime_make_array,
            "runtime_index": runtime_index,
            "runtime_set_index": runtime_set_index,
            "runtime_make_struct": runtime_make_struct,
            "runtime_get_field": runtime_get_field,
            "runtime_set_field": runtime_set_field,
            "runtime_call_builtin": runtime_call_builtin,
            "runtime_print": runtime_print,
            "runtime_null": runtime_null,
            "Op": Op,
        }
        try:
            compiled = compile(source, f"<tinyone-jit-{key}>", "exec")
            exec(compiled, namespace)  # noqa: S102
        except Exception as error:
            raise CompileError(f"JIT compilation failed: {error}") from error

        function = namespace.get("_tinyone_jit")
        if not isinstance(function, FunctionType):
            raise CompileError("JIT compiler failed to produce a callable")

        self._cache[key] = function
        LOGGER.debug(
            "jit compiled",
            extra={"fingerprint": key, "instructions": len(program.code)},
        )
        return function

    def _build_source(self, program: Program) -> str:
        if self._can_emit_straightline(program):
            return self._build_straightline_source(program)
        return self._build_dispatch_source(program)

    @staticmethod
    def _can_emit_straightline(program: Program) -> bool:
        if program.functions:
            return False
        unsupported = {Op.JUMP, Op.JUMP_IF_ZERO, Op.CALL, Op.RETURN}
        return not any(instr.op in unsupported for instr in program.code)

    def _build_straightline_source(self, program: Program) -> str:
        """
        Emit a Python function that executes branch-free main bytecode using
        local variables instead of a simulated stack list.

        Correctness contract: BytecodeVerifier has already confirmed that sp
        never goes negative and is exactly 0 at HALT.  We assert that contract
        here defensively during codegen.
        """
        lines = [
            "def _tinyone_jit(memory, stdout, context=None):",
            "    if context is None:",
            "        context = TinyRuntimeContext()",
        ]
        sp = 0

        def slot(depth: int) -> str:
            return f"_s{depth}"

        for i, instr in enumerate(program.code):
            op = instr.op
            arg = instr.arg

            if op == Op.PUSH_INT:
                lines.append(f"    {slot(sp)} = {arg!r}")
                sp += 1

            elif op == Op.PUSH_NULL:
                lines.append(f"    {slot(sp)} = runtime_null()")
                sp += 1

            elif op == Op.PUSH_STRING:
                lines.append(f"    {slot(sp)} = context.heap.alloc_string({program.strings[arg]!r})")
                sp += 1

            elif op == Op.LOAD:
                lines.append(f"    {slot(sp)} = memory.load({arg})")
                sp += 1

            elif op == Op.STORE:
                if sp < 1:
                    raise CompileError(
                        f"JIT codegen: STORE at index {i} with empty stack (verifier bug)"
                    )
                sp -= 1
                lines.append(f"    memory.store({arg}, {slot(sp)})")

            elif op == Op.ADD:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: ADD at index {i} requires depth>=2, got {sp} (verifier bug)"
                    )
                # lhs = slot(sp-2), rhs = slot(sp-1), result -> slot(sp-2)
                lines.append(
                    f"    {slot(sp - 2)} = runtime_add({slot(sp - 2)}, {slot(sp - 1)})"
                )
                sp -= 1

            elif op == Op.SUB:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: SUB at index {i} requires depth>=2, got {sp} (verifier bug)"
                    )
                lines.append(
                    f"    {slot(sp - 2)} = runtime_sub({slot(sp - 2)}, {slot(sp - 1)})"
                )
                sp -= 1

            elif op == Op.MUL:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: MUL at index {i} requires depth>=2, got {sp} (verifier bug)"
                    )
                lines.append(
                    f"    {slot(sp - 2)} = runtime_mul({slot(sp - 2)}, {slot(sp - 1)})"
                )
                sp -= 1

            elif op == Op.DIV:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: DIV at index {i} requires depth>=2, got {sp} (verifier bug)"
                    )
                lines.append(
                    f"    {slot(sp - 2)} = checked_div({slot(sp - 2)}, {slot(sp - 1)})"
                )
                sp -= 1

            elif op == Op.NEG:
                if sp < 1:
                    raise CompileError(
                        f"JIT codegen: NEG at index {i} with empty stack (verifier bug)"
                    )
                lines.append(f"    {slot(sp - 1)} = runtime_neg({slot(sp - 1)})")

            elif op in (Op.LT, Op.LTE, Op.GT, Op.GTE, Op.EQ, Op.NE):
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: {op.name} at index {i} requires depth>=2, got {sp} "
                        "(verifier bug)"
                    )
                lines.append(
                    f"    {slot(sp - 2)} = runtime_compare(Op.{op.name}, "
                    f"{slot(sp - 2)}, {slot(sp - 1)})"
                )
                sp -= 1

            elif op == Op.MAKE_ARRAY:
                if sp < arg:
                    raise CompileError(
                        f"JIT codegen: MAKE_ARRAY at index {i} requires depth>={arg}, got {sp} "
                        "(verifier bug)"
                    )
                values = ", ".join(slot(depth) for depth in range(sp - arg, sp))
                lines.append(
                    f"    {slot(sp - arg)} = runtime_make_array(context, [{values}])"
                )
                sp = sp - arg + 1

            elif op == Op.INDEX:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: INDEX at index {i} requires depth>=2, got {sp} "
                        "(verifier bug)"
                    )
                lines.append(
                    f"    {slot(sp - 2)} = runtime_index(context, {slot(sp - 2)}, {slot(sp - 1)})"
                )
                sp -= 1

            elif op == Op.SET_INDEX:
                if sp < 3:
                    raise CompileError(
                        f"JIT codegen: SET_INDEX at index {i} requires depth>=3, got {sp} "
                        "(verifier bug)"
                    )
                lines.append(
                    f"    runtime_set_index(context, {slot(sp - 3)}, {slot(sp - 2)}, "
                    f"{slot(sp - 1)})"
                )
                sp -= 3

            elif op == Op.MAKE_STRUCT:
                if sp < instr.arg2:
                    raise CompileError(
                        f"JIT codegen: MAKE_STRUCT at index {i} requires depth>={instr.arg2}, "
                        f"got {sp} (verifier bug)"
                    )
                struct = program.structs[arg]
                values = ", ".join(slot(depth) for depth in range(sp - instr.arg2, sp))
                lines.append(
                    f"    {slot(sp - instr.arg2)} = runtime_make_struct("
                    f"context, {struct.name!r}, {struct.fields!r}, [{values}])"
                )
                sp = sp - instr.arg2 + 1

            elif op == Op.GET_FIELD:
                if sp < 1:
                    raise CompileError(
                        f"JIT codegen: GET_FIELD at index {i} with empty stack "
                        "(verifier bug)"
                    )
                lines.append(
                    f"    {slot(sp - 1)} = runtime_get_field("
                    f"context, {slot(sp - 1)}, {program.fields[arg]!r})"
                )

            elif op == Op.SET_FIELD:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: SET_FIELD at index {i} requires depth>=2, got {sp} "
                        "(verifier bug)"
                    )
                lines.append(
                    f"    runtime_set_field(context, {slot(sp - 2)}, "
                    f"{program.fields[arg]!r}, {slot(sp - 1)})"
                )
                sp -= 2

            elif op == Op.BUILTIN:
                if sp < instr.arg2:
                    raise CompileError(
                        f"JIT codegen: BUILTIN at index {i} requires depth>={instr.arg2}, "
                        f"got {sp} (verifier bug)"
                    )
                args = ", ".join(slot(depth) for depth in range(sp - instr.arg2, sp))
                lines.append(
                    f"    {slot(sp - instr.arg2)} = runtime_call_builtin("
                    f"context, {arg}, [{args}])"
                )
                sp = sp - instr.arg2 + 1

            elif op == Op.PRINT:
                if sp < 1:
                    raise CompileError(
                        f"JIT codegen: PRINT at index {i} with empty stack (verifier bug)"
                    )
                lines.append(f"    runtime_print(context, stdout, {slot(sp - 1)})")
                sp -= 1

            elif op == Op.HALT:
                # BytecodeVerifier guarantees sp == 0 here.  No runtime check
                # is needed; we just return.
                lines.append("    return")

            else:
                raise CompileError(f"JIT codegen: cannot emit unknown opcode {op!r}")

        # Safety: if HALT was not the final instruction the compiler emitted
        # (shouldn't happen) ensure the function still returns.
        if not lines[-1].strip().startswith("return"):
            lines.append("    return")

        return "\n".join(lines) + "\n"

    def _build_dispatch_source(self, program: Program) -> str:
        lines = [
            "def _tinyone_jit(memory, stdout, context=None):",
            "    if context is None:",
            "        context = TinyRuntimeContext()",
            "    return _tinyone_main(memory, stdout, context)",
            "",
            "def _tinyone_call(function_index, args, stdout, context):",
        ]
        if not program.functions:
            lines.append(
                "    raise RuntimeTinyOneError(f'Invalid function index {function_index}')"
            )
        else:
            for index, function in enumerate(program.functions):
                prefix = "if" if index == 0 else "elif"
                lines.append(f"    {prefix} function_index == {index}:")
                lines.append(f"        return _tinyone_func_{index}(args, stdout, context)")
            lines.append(
                "    raise RuntimeTinyOneError(f'Invalid function index {function_index}')"
            )
        lines.append("")

        for index, function in enumerate(program.functions):
            lines.extend(self._build_dispatch_function(index, function, program))
            lines.append("")

        lines.extend(
            self._build_dispatch_chunk(
                "_tinyone_main",
                program.code,
                program.slot_count,
                program=program,
                param_count=0,
                chunk_name="main",
                use_existing_memory=True,
            )
        )
        return "\n".join(lines) + "\n"

    def _build_dispatch_function(
        self, index: int, function: Function, program: Program
    ) -> list[str]:
        return self._build_dispatch_chunk(
            f"_tinyone_func_{index}",
            function.code,
            function.slot_count,
            program=program,
            param_count=function.param_count,
            chunk_name=function.name,
            use_existing_memory=False,
        )

    def _build_dispatch_chunk(
        self,
        function_name: str,
        code: tuple[Instr, ...],
        slot_count: int,
        *,
        program: Program | None,
        param_count: int,
        chunk_name: str,
        use_existing_memory: bool,
    ) -> list[str]:
        if use_existing_memory:
            lines = [f"def {function_name}(memory, stdout, context):"]
        else:
            lines = [f"def {function_name}(args, stdout, context):"]
            lines.append(f"    if len(args) != {param_count}:")
            lines.append(
                f"        raise RuntimeTinyOneError(\"Function {chunk_name!r} expects "
                f"{param_count} argument(s)\")"
            )
            lines.append(f"    memory = TinyMemory({slot_count})")
            for slot in range(param_count):
                lines.append(f"    memory.store({slot}, args[{slot}])")

        lines.extend(
            [
                "    stack = []",
                "    pc = 0",
                "    while True:",
            ]
        )
        for index, instr in enumerate(code):
            lines.append(f"        if pc == {index}:")
            lines.extend(self._build_dispatch_instr(instr, index, program))
        lines.append(
            f"        raise RuntimeTinyOneError(\"Invalid program counter in {chunk_name}\")"
        )
        return lines
    
    def _build_dispatch_instr(
        self,
        instr: Instr,
        index: int,
        program: Program | None = None,
    ) -> list[str]:
        emitter = _DISPATCH_EMITTERS.get(instr.op)
        if emitter is None:
            raise CompileError(f"JIT codegen: cannot emit unknown opcode {instr.op!r}")
        return emitter(instr, index, program)
    
    @staticmethod
    def _python_comparison_operator(op: Op) -> str:
        result = _CMP_PYTHON_OP.get(op)
        if result is None:
            raise CompileError(f"JIT codegen: unsupported comparison opcode {op!r}")
        return result

# Instruction emitters for the dispatch-loop JIT path.
# Each function receives (instr, index, program) and returns
# the list of Python source lines for that opcode.
_InstrEmitter = Callable[["Instr", int, "Program | None"], list[str]]


def _emit_push_int(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        f"            stack.append({instr.arg!r})",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_push_null(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            stack.append(runtime_null())",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_push_string(instr: Instr, index: int, program: Program | None) -> list[str]:
    if program is None:
        raise CompileError("JIT codegen: missing program metadata for string literal")
    return [
        f"            stack.append(context.heap.alloc_string({program.strings[instr.arg]!r}))",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_load(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        f"            stack.append(memory.load({instr.arg}))",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_store(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        f"            memory.store({instr.arg}, stack.pop())",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_add(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            rhs = stack.pop()",
        "            stack[-1] = runtime_add(stack[-1], rhs)",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_sub(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            rhs = stack.pop()",
        "            stack[-1] = runtime_sub(stack[-1], rhs)",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_mul(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            rhs = stack.pop()",
        "            stack[-1] = runtime_mul(stack[-1], rhs)",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_div(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            rhs = stack.pop()",
        "            stack[-1] = checked_div(stack[-1], rhs)",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_neg(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            stack[-1] = runtime_neg(stack[-1])",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_compare(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            rhs = stack.pop()",
        f"            stack[-1] = runtime_compare(Op.{instr.op.name}, stack[-1], rhs)",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_jump(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [f"            pc = {instr.arg}", "            continue"]


def _emit_jump_if_zero(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        f"            pc = {instr.arg} if runtime_is_false(stack.pop()) else {index + 1}",
        "            continue",
    ]


def _emit_call(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        f"            args = [stack.pop() for _ in range({instr.arg2})]",
        "            args.reverse()",
        f"            stack.append(_tinyone_call({instr.arg}, args, stdout, context))",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_make_array(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        f"            values = [stack.pop() for _ in range({instr.arg})]",
        "            values.reverse()",
        "            stack.append(runtime_make_array(context, values))",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_index(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            index = stack.pop()",
        "            container = stack.pop()",
        "            stack.append(runtime_index(context, container, index))",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_set_index(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            value = stack.pop()",
        "            index = stack.pop()",
        "            container = stack.pop()",
        "            runtime_set_index(context, container, index, value)",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_make_struct(instr: Instr, index: int, program: Program | None) -> list[str]:
    if program is None:
        raise CompileError("JIT codegen: missing program metadata for struct")
    struct = program.structs[instr.arg]
    return [
        f"            values = [stack.pop() for _ in range({instr.arg2})]",
        "            values.reverse()",
        f"            stack.append(runtime_make_struct(context, {struct.name!r}, "
        f"{struct.fields!r}, values))",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_get_field(instr: Instr, index: int, program: Program | None) -> list[str]:
    if program is None:
        raise CompileError("JIT codegen: missing program metadata for field read")
    return [
        f"            stack[-1] = runtime_get_field(context, stack[-1], "
        f"{program.fields[instr.arg]!r})",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_set_field(instr: Instr, index: int, program: Program | None) -> list[str]:
    if program is None:
        raise CompileError("JIT codegen: missing program metadata for field write")
    return [
        "            value = stack.pop()",
        "            target = stack.pop()",
        f"            runtime_set_field(context, target, {program.fields[instr.arg]!r}, value)",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_builtin(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        f"            args = [stack.pop() for _ in range({instr.arg2})]",
        "            args.reverse()",
        f"            stack.append(runtime_call_builtin(context, {instr.arg}, args))",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_return(instr: Instr, index: int, program: Program | None) -> list[str]:
    return ["            return stack.pop()"]


def _emit_print(instr: Instr, index: int, program: Program | None) -> list[str]:
    return [
        "            runtime_print(context, stdout, stack.pop())",
        f"            pc = {index + 1}",
        "            continue",
    ]


def _emit_halt(instr: Instr, index: int, program: Program | None) -> list[str]:
    return ["            return"]


_DISPATCH_EMITTERS: Final[dict[Op, _InstrEmitter]] = {
    Op.PUSH_INT:    _emit_push_int,
    Op.PUSH_NULL:   _emit_push_null,
    Op.PUSH_STRING: _emit_push_string,
    Op.LOAD:        _emit_load,
    Op.STORE:       _emit_store,
    Op.ADD:         _emit_add,
    Op.SUB:         _emit_sub,
    Op.MUL:         _emit_mul,
    Op.DIV:         _emit_div,
    Op.NEG:         _emit_neg,
    Op.LT:          _emit_compare,
    Op.LTE:         _emit_compare,
    Op.GT:          _emit_compare,
    Op.GTE:         _emit_compare,
    Op.EQ:          _emit_compare,
    Op.NE:          _emit_compare,
    Op.JUMP:        _emit_jump,
    Op.JUMP_IF_ZERO: _emit_jump_if_zero,
    Op.CALL:        _emit_call,
    Op.MAKE_ARRAY:  _emit_make_array,
    Op.INDEX:       _emit_index,
    Op.SET_INDEX:   _emit_set_index,
    Op.MAKE_STRUCT: _emit_make_struct,
    Op.GET_FIELD:   _emit_get_field,
    Op.SET_FIELD:   _emit_set_field,
    Op.BUILTIN:     _emit_builtin,
    Op.RETURN:      _emit_return,
    Op.PRINT:       _emit_print,
    Op.HALT:        _emit_halt,
}


def compile_source(source: str, *, filename: str = "<source>") -> Program:
    """
    Full compilation pipeline:
        Compiler  ->  PeepholeOptimizer  ->  BytecodeVerifier  ->  Program

    The returned Program is optimized and verified.  VM and JIT both receive
    the same bytecode, guaranteeing semantic equivalence.
    """
    program = Compiler(source, filename=filename).compile()
    program = PeepholeOptimizer.optimize(program)
    BytecodeVerifier.verify(program)
    LOGGER.debug(
        "compile_source complete",
        extra={"instructions": len(program.code), "slots": program.slot_count},
    )
    return program


def _module_name_from_filename(filename: str) -> str:
    return _sanitize_identifier(Path(filename).stem or "module")


def _module_name_from_import(import_path: str, filename: str) -> str:
    if _looks_like_module_key(import_path):
        return _sanitize_identifier(import_path)
    return _module_name_from_filename(filename)


def _unique_module_name(shared: CompilerSharedState, base_name: str, filename: str) -> str:
    existing_owner = shared.module_name_owners.get(base_name)
    if existing_owner is None or existing_owner == filename:
        shared.module_name_owners[base_name] = filename
        return base_name
    suffix = hashlib.blake2b(filename.encode("utf-8"), digest_size=4).hexdigest()
    name = f"{base_name}_{suffix}"
    while name in shared.module_name_owners and shared.module_name_owners[name] != filename:
        suffix = hashlib.blake2b(f"{filename}:{suffix}".encode("utf-8"), digest_size=4).hexdigest()
        name = f"{base_name}_{suffix}"
    shared.module_name_owners[name] = filename
    return name


def _default_import_alias(import_path: str) -> str:
    if _looks_like_module_key(import_path):
        return _sanitize_identifier(import_path)
    return _sanitize_identifier(Path(import_path).stem or "module")


def _looks_like_module_key(import_path: str) -> bool:
    return (
        "/" not in import_path
        and "\\" not in import_path
        and not import_path.startswith(".")
        and "." not in import_path
    )


def _sanitize_identifier(text: str) -> str:
    chars = [char if char == "_" or char.isalnum() else "_" for char in text]
    sanitized = "".join(chars).strip("_")
    if not sanitized or sanitized[0].isdigit():
        sanitized = f"module_{sanitized}"
    return sanitized


def _resolve_manifest_import(base: Path, import_path: str) -> Path | None:
    if not _looks_like_module_key(import_path):
        return None
    for directory in (base, *base.parents):
        manifest_path = directory / "tinyone.json"
        if not manifest_path.exists():
            continue
        try:
            data = json.loads(manifest_path.read_text(encoding="utf-8"))
        except OSError as error:
            raise CompileError(f"Package manifest read error: {error}") from error
        except json.JSONDecodeError as error:
            raise CompileError(f"Package manifest JSON error: {error}") from error
        modules = data.get("modules") if isinstance(data, dict) else None
        if not isinstance(modules, dict):
            raise CompileError(f"Package manifest {manifest_path} must contain a modules object")
        target = modules.get(import_path)
        if target is None:
            continue
        if not isinstance(target, str):
            raise CompileError(
                f"Package manifest module {import_path!r} in {manifest_path} must be a string"
            )
        return (directory / target).resolve()
    return None


def _resolve_import(from_filename: str, import_path: str) -> tuple[str, str]:
    base = Path(from_filename).resolve().parent
    path = _resolve_manifest_import(base, import_path)
    if path is None:
        path = (base / import_path).resolve()
    try:
        return str(path), path.read_text(encoding="utf-8")
    except OSError as error:
        raise CompileError(f"Import error: {error}") from error


def compile_file(path: str | Path) -> Program:
    source_path = Path(path).resolve()
    try:
        source = source_path.read_text(encoding="utf-8")
    except OSError as error:
        raise CompileError(f"File error: {error}") from error
    imported = {str(source_path)}
    program = Compiler(
        source,
        filename=str(source_path),
        resolver=_resolve_import,
        imported=imported,
    ).compile()
    program = PeepholeOptimizer.optimize(program)
    BytecodeVerifier.verify(program)
    return program


def load_artifact(path: str | Path) -> Program:
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except OSError as error:
        raise CompileError(f"Artifact read error: {error}") from error
    except json.JSONDecodeError as error:
        raise CompileError(f"Artifact JSON error: {error}") from error
    return Program.from_artifact(data)


def write_artifact(program: Program, path: str | Path) -> None:
    try:
        Path(path).write_text(
            json.dumps(program.to_artifact(), indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    except OSError as error:
        raise CompileError(f"Artifact write error: {error}") from error


def run_program(
    program: Program,
    *,
    mode: str,
    stdout: TextIO,
    inputs: Iterable[object] | None = None,
    sys_args: Iterable[str] | None = None,
    sys_env: dict[str, str] | None = None,
) -> TinyMemory:
    memory = TinyMemory(program.slot_count)
    context = TinyRuntimeContext(inputs, sys_args=sys_args, sys_env=sys_env)

    if mode == "vm":
        VM(program, memory, stdout, context).run()
    elif mode == "jit":
        JitCache().compile(program)(memory, stdout, context)
    else:
        raise ValueError(f"Unsupported mode {mode!r}")

    return memory


def run_source(
    source: str,
    *,
    mode: str,
    stdout: TextIO,
    inputs: Iterable[object] | None = None,
) -> TinyMemory:
    return run_program(compile_source(source), mode=mode, stdout=stdout, inputs=inputs)


def _configure_logging(verbose: bool) -> None:
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.WARNING,
        format="%(levelname)s %(name)s %(message)s",
    )


def parse_args(argv: Iterable[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="TinyOne compiler/VM/JIT")
    parser.add_argument("path", nargs="?", help="TinyOne source file")
    parser.add_argument(
        "--mode",
        choices=("jit", "vm"),
        default="jit",
        help="execution backend; jit compiles bytecode into Python locals",
    )
    parser.add_argument("--check", action="store_true", help="compile and verify without running")
    parser.add_argument("--emit-bytecode", metavar="PATH", help="write a JSON bytecode artifact")
    parser.add_argument("--run-bytecode", metavar="PATH", help="run a JSON bytecode artifact")
    parser.add_argument(
        "--input",
        action="append",
        default=[],
        help="append one deterministic input value for read/read_int/read_str",
    )
    parser.add_argument(
        "--stdin",
        action="store_true",
        help="append stdin lines to the deterministic input queue",
    )
    parser.add_argument("--verbose", action="store_true", help="enable debug logging")
    return parser.parse_args(list(argv))


def main(argv: list[str]) -> int:
    args = parse_args(argv[1:])
    _configure_logging(args.verbose)

    try:
        inputs = list(args.input)
        if args.stdin:
            inputs.extend(line.rstrip("\n") for line in sys.stdin)

        if args.run_bytecode is not None:
            program = load_artifact(args.run_bytecode)
        else:
            if args.path is None:
                print("File error: a source path is required", file=sys.stderr)
                return 1
            program = compile_file(args.path)

        if args.emit_bytecode is not None:
            write_artifact(program, args.emit_bytecode)
        if not args.check:
            run_program(program, mode=args.mode, stdout=sys.stdout, inputs=inputs)
        return 0
    except TinyOneError as error:
        print(f"TinyOne error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
