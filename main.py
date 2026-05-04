#!/usr/bin/env python3
"""
TinyOne: single-file stdlib-only compiler/VM/JIT implementation.

Language:
    let x = 1 + 2 * 3
    while x < 10 { let x = x + 1 }
    fn double(n) { return n * 2 }
    print x

Design constraints:
    - Python stdlib only
    - Single file
    - Maintainable Python implementation

Runtime model:
    Source -> tokens -> bytecode -> [peephole] -> [verify] -> VM or JIT.

Memory model:
    TinyMemory is an arena of integer slots addressed by Slot handles. Runtime
    code does not use a Python dict for variable lookup; dict usage is limited
    to compile-time symbol interning.

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
import logging
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


class TokenKind(IntEnum):
    INT = 1
    IDENT = 2
    LET = 3
    PRINT = 4
    FN = 5
    RETURN = 6
    WHILE = 7
    PLUS = 8
    MINUS = 9
    STAR = 10
    SLASH = 11
    EQUAL = 12
    EQEQ = 13
    BANG_EQUAL = 14
    LT = 15
    LTE = 16
    GT = 17
    GTE = 18
    LPAREN = 19
    RPAREN = 20
    LBRACE = 21
    RBRACE = 22
    COMMA = 23
    EOF = 24


@dataclass(frozen=True, slots=True)
class Token:
    kind: TokenKind
    text: str
    pos: int


_KEYWORDS: Final[dict[str, TokenKind]] = {
    "let": TokenKind.LET,
    "print": TokenKind.PRINT,
    "fn": TokenKind.FN,
    "return": TokenKind.RETURN,
    "while": TokenKind.WHILE,
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
    ",": TokenKind.COMMA,
}


class Lexer:
    """Hand-written lexer optimized for one pass over the source string."""

    __slots__ = ("_source", "_length", "_pos")

    def __init__(self, source: str) -> None:
        self._source = source
        self._length = len(source)
        self._pos = 0

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

            if "0" <= ch <= "9":
                start = pos
                pos += 1
                while pos < length and "0" <= source[pos] <= "9":
                    pos += 1
                append(Token(TokenKind.INT, source[start:pos], start))
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
                append(Token(_KEYWORDS.get(text, TokenKind.IDENT), text, start))
                continue

            if pos + 1 < length:
                pair = source[pos : pos + 2]
                kind = _TWO_CHAR_TOKENS.get(pair)
                if kind is not None:
                    append(Token(kind, pair, pos))
                    pos += 2
                    continue

            kind = _SINGLE_CHAR_TOKENS.get(ch)
            if kind is None:
                raise CompileError(f"Unexpected character {ch!r} at position {pos}")
            append(Token(kind, ch, pos))
            pos += 1

        append(Token(TokenKind.EOF, "", pos))
        self._pos = pos
        return tokens


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
class Program:
    code: tuple[Instr, ...]
    slot_count: int
    names: tuple[str, ...]
    functions: tuple[Function, ...] = ()

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
        return hasher.hexdigest()

    @staticmethod
    def _hash_code(hasher: object, code: tuple[Instr, ...]) -> None:
        for instr in code:
            hasher.update(int(instr.op).to_bytes(2, "little", signed=False))
            hasher.update(int(instr.arg).to_bytes(16, "little", signed=True))
            hasher.update(int(instr.arg2).to_bytes(16, "little", signed=True))


class SymbolTable:
    """Compile-time symbol interner. Runtime uses slots, not names."""

    __slots__ = ("_slots", "_names")

    def __init__(self) -> None:
        self._slots: dict[str, Slot] = {}
        self._names: list[str] = []

    def define_or_get(self, name: str) -> Slot:
        existing = self._slots.get(name)
        if existing is not None:
            return existing
        slot = Slot(len(self._names))
        self._slots[name] = slot
        self._names.append(name)
        return slot

    def get(self, name: str, pos: int) -> Slot:
        slot = self._slots.get(name)
        if slot is None:
            raise CompileError(f"Undefined variable {name!r} at position {pos}")
        return slot

    @property
    def slot_count(self) -> int:
        return len(self._names)

    @property
    def names(self) -> tuple[str, ...]:
        return tuple(self._names)


class Compiler:
    """Recursive-descent parser that emits stack-machine bytecode."""

    __slots__ = (
        "_tokens",
        "_index",
        "_current",
        "_symbols",
        "_code",
        "_function_indexes",
        "_function_names",
        "_functions",
        "_in_function",
    )

    def __init__(self, source: str) -> None:
        self._tokens = Lexer(source).tokenize()
        self._index = 0
        self._current = self._tokens[0]
        self._symbols = SymbolTable()
        self._code: list[Instr] = []
        self._function_indexes: dict[str, int] = {}
        self._function_names: list[str] = []
        self._functions: list[Function | None] = []
        self._in_function = False

    def compile(self) -> Program:
        while self._current.kind != TokenKind.EOF:
            if self._current.kind == TokenKind.FN:
                self._function_definition()
            else:
                self._statement()
        self._emit(Op.HALT)
        return Program(
            tuple(self._code),
            self._symbols.slot_count,
            self._symbols.names,
            self._resolved_functions(),
        )

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
        if kind == TokenKind.RETURN:
            self._return_statement()
            return
        if kind == TokenKind.FN:
            raise CompileError(
                "Function definitions are only allowed at top level "
                f"at position {self._current.pos}"
            )
        raise CompileError(f"Expected statement at position {self._current.pos}")

    def _let_statement(self) -> None:
        self._eat(TokenKind.LET)
        name = self._current.text
        name_pos = self._current.pos
        self._eat(TokenKind.IDENT)
        self._eat(TokenKind.EQUAL)
        self._expression()
        slot = self._symbols.define_or_get(name)
        LOGGER.debug("compiled let", extra={"name": name, "slot": int(slot), "pos": name_pos})
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
        self._block()
        self._emit(Op.JUMP, loop_start)
        self._patch(exit_jump, len(self._code))

    def _return_statement(self) -> None:
        if not self._in_function:
            raise CompileError(f"Return outside function at position {self._current.pos}")
        self._eat(TokenKind.RETURN)
        self._expression()
        self._emit(Op.RETURN)

    def _function_definition(self) -> None:
        self._eat(TokenKind.FN)
        name = self._current.text
        name_pos = self._current.pos
        self._eat(TokenKind.IDENT)
        function_index = self._function_index(name)
        if self._functions[function_index] is not None:
            raise CompileError(f"Function {name!r} is already defined at position {name_pos}")

        function_symbols = SymbolTable()
        self._eat(TokenKind.LPAREN)
        param_count = 0
        if self._current.kind != TokenKind.RPAREN:
            while True:
                param_name = self._current.text
                param_pos = self._current.pos
                self._eat(TokenKind.IDENT)
                slot = function_symbols.define_or_get(param_name)
                if int(slot) != param_count:
                    raise CompileError(
                        f"Duplicate parameter {param_name!r} at position {param_pos}"
                    )
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
                name,
                param_count,
                tuple(self._code),
                self._symbols.slot_count,
                self._symbols.names,
            )
        finally:
            self._symbols = previous_symbols
            self._code = previous_code
            self._in_function = previous_in_function

        self._functions[function_index] = function
        LOGGER.debug(
            "compiled function",
            extra={"name": name, "index": function_index, "params": param_count},
        )

    def _block(self) -> None:
        self._eat(TokenKind.LBRACE)
        while self._current.kind != TokenKind.RBRACE:
            if self._current.kind == TokenKind.EOF:
                raise CompileError(f"Unterminated block at position {self._current.pos}")
            self._statement()
        self._eat(TokenKind.RBRACE)

    def _expression(self) -> None:
        self._comparison()

    def _comparison(self) -> None:
        self._additive()
        comparison_ops = {
            TokenKind.LT: Op.LT,
            TokenKind.LTE: Op.LTE,
            TokenKind.GT: Op.GT,
            TokenKind.GTE: Op.GTE,
            TokenKind.EQEQ: Op.EQ,
            TokenKind.BANG_EQUAL: Op.NE,
        }
        while self._current.kind in comparison_ops:
            op = self._current.kind
            self._eat(op)
            self._additive()
            self._emit(comparison_ops[op])

    def _additive(self) -> None:
        self._term()
        while self._current.kind in (TokenKind.PLUS, TokenKind.MINUS):
            op = self._current.kind
            self._eat(op)
            self._term()
            self._emit(Op.ADD if op == TokenKind.PLUS else Op.SUB)

    def _term(self) -> None:
        self._factor()
        while self._current.kind in (TokenKind.STAR, TokenKind.SLASH):
            op = self._current.kind
            self._eat(op)
            self._factor()
            self._emit(Op.MUL if op == TokenKind.STAR else Op.DIV)

    def _factor(self) -> None:
        token = self._current
        kind = token.kind

        if kind == TokenKind.INT:
            self._eat(TokenKind.INT)
            self._emit(Op.PUSH_INT, int(token.text))
            return

        if kind == TokenKind.IDENT:
            self._eat(TokenKind.IDENT)
            if self._current.kind == TokenKind.LPAREN:
                self._call_expression(token.text)
            else:
                slot = self._symbols.get(token.text, token.pos)
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

        raise CompileError(f"Expected expression at position {token.pos}")

    def _call_expression(self, name: str) -> None:
        function_index = self._function_index(name)
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

    def _eat(self, kind: TokenKind) -> None:
        if self._current.kind != kind:
            raise CompileError(
                f"Expected {kind.name}, got {self._current.kind.name} "
                f"at position {self._current.pos}"
            )
        self._index += 1
        self._current = self._tokens[self._index]

    def _emit(self, op: Op, arg: int = 0, arg2: int = 0) -> None:
        self._code.append(Instr(op, arg, arg2))

    def _emit_placeholder(self, op: Op) -> int:
        index = len(self._code)
        self._code.append(Instr(op, -1))
        return index

    def _patch(self, index: int, arg: int) -> None:
        instr = self._code[index]
        self._code[index] = Instr(instr.op, arg, instr.arg2)

    def _function_index(self, name: str) -> int:
        existing = self._function_indexes.get(name)
        if existing is not None:
            return existing
        index = len(self._function_names)
        self._function_indexes[name] = index
        self._function_names.append(name)
        self._functions.append(None)
        return index

    def _resolved_functions(self) -> tuple[Function, ...]:
        missing = [
            self._function_names[index]
            for index, function in enumerate(self._functions)
            if function is None
        ]
        if missing:
            joined = ", ".join(repr(name) for name in missing)
            raise CompileError(f"Undefined function(s): {joined}")
        return tuple(function for function in self._functions if function is not None)


# ---------------------------------------------------------------------------
# Stack effects for each opcode.  Used by BytecodeVerifier and kept as a
# module-level constant so both components share the same source of truth.
# ---------------------------------------------------------------------------
_STACK_EFFECTS: Final[dict[Op, int]] = {
    Op.PUSH_INT: 1,
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
}


class BytecodeVerifier:
    """
    O(n) static stack-depth checker.

    Simulates stack depth changes across the reachable control-flow graph
    without executing anything. Raises CompileError on:
      - negative depth mid-sequence  (stack underflow)
      - non-zero depth at HALT       (stack imbalance)
      - non-one depth at RETURN      (function stack imbalance)
      - inconsistent depth at a jump target
      - unrecognised opcode          (compiler/optimizer bug)

    By catching structural errors before execution, the VM and JIT can omit
    redundant runtime guards.  The JIT in particular no longer needs the
    end-of-function stack-imbalance check because it is a compile-time
    invariant after this pass.
    """

    @classmethod
    def verify(cls, program: Program) -> None:
        cls._verify_chunk(
            program.code,
            program.slot_count,
            program.functions,
            "main",
            final_op=Op.HALT,
        )
        for index, function in enumerate(program.functions):
            cls._verify_chunk(
                function.code,
                function.slot_count,
                program.functions,
                f"function {function.name!r} (index {index})",
                final_op=Op.RETURN,
            )

    @classmethod
    def _verify_chunk(
        cls,
        code: tuple[Instr, ...],
        slot_count: int,
        functions: tuple[Function, ...],
        chunk_name: str,
        *,
        final_op: Op,
    ) -> None:
        if not code:
            raise CompileError(f"Verifier: {chunk_name} has no bytecode")
        if code[-1].op != final_op:
            raise CompileError(
                f"Verifier: {chunk_name} must end with {final_op.name}, got {code[-1].op.name}"
            )

        depths: dict[int, int] = {}
        worklist: list[tuple[int, int]] = []

        def enqueue(target: int, depth: int, source_index: int) -> None:
            if target < 0 or target >= len(code):
                raise CompileError(
                    f"Verifier: jump/fallthrough from instruction {source_index} "
                    f"in {chunk_name} targets invalid instruction {target}"
                )
            existing = depths.get(target)
            if existing is not None:
                if existing != depth:
                    raise CompileError(
                        f"Verifier: inconsistent stack depth at instruction {target} "
                        f"in {chunk_name}: {existing} vs {depth}"
                    )
                return
            depths[target] = depth
            worklist.append((target, depth))

        enqueue(0, 0, 0)

        while worklist:
            i, depth = worklist.pop()
            instr = code[i]
            op = instr.op
            arg = instr.arg
            arg2 = instr.arg2

            if op == Op.LOAD or op == Op.STORE:
                cls._verify_slot(arg, slot_count, i, chunk_name)

            effect = _STACK_EFFECTS.get(instr.op)
            if effect is not None:
                next_depth = depth + effect
                if next_depth < 0:
                    raise CompileError(
                        f"Verifier: stack underflow at instruction {i} in {chunk_name} "
                        f"({op.name}, cumulative depth={next_depth})"
                    )
                cls._enqueue_next(code, enqueue, i, next_depth, chunk_name)
                continue

            if op == Op.JUMP:
                enqueue(arg, depth, i)
                continue

            if op == Op.JUMP_IF_ZERO:
                if depth < 1:
                    raise CompileError(
                        f"Verifier: stack underflow at instruction {i} in {chunk_name} "
                        f"({op.name}, cumulative depth={depth - 1})"
                    )
                next_depth = depth - 1
                enqueue(arg, next_depth, i)
                cls._enqueue_next(code, enqueue, i, next_depth, chunk_name)
                continue

            if op == Op.CALL:
                cls._verify_call(arg, arg2, functions, i, chunk_name)
                next_depth = depth - arg2 + 1
                if next_depth < 0:
                    raise CompileError(
                        f"Verifier: stack underflow at instruction {i} in {chunk_name} "
                        f"({op.name}, cumulative depth={next_depth})"
                    )
                cls._enqueue_next(code, enqueue, i, next_depth, chunk_name)
                continue

            if op == Op.RETURN:
                if depth != 1:
                    raise CompileError(
                        f"Verifier: RETURN in {chunk_name} requires one value "
                        f"on the stack, got {depth}"
                    )
                continue

            if op == Op.HALT:
                if depth != 0:
                    raise CompileError(
                        f"Verifier: HALT in {chunk_name} requires empty stack, got {depth}"
                    )
                continue

            raise CompileError(f"Verifier: unknown opcode {op!r} at index {i} in {chunk_name}")

    @staticmethod
    def _enqueue_next(
        code: tuple[Instr, ...],
        enqueue: Callable[[int, int, int], None],
        index: int,
        depth: int,
        chunk_name: str,
    ) -> None:
        target = index + 1
        if target >= len(code):
            raise CompileError(
                f"Verifier: {chunk_name} falls off the end after instruction {index}"
            )
        enqueue(target, depth, index)

    @staticmethod
    def _verify_slot(slot: int, slot_count: int, index: int, chunk_name: str) -> None:
        if slot < 0 or slot >= slot_count:
            raise CompileError(
                f"Verifier: invalid slot {slot} at instruction {index} in {chunk_name}"
            )

    @staticmethod
    def _verify_call(
        function_index: int,
        arg_count: int,
        functions: tuple[Function, ...],
        index: int,
        chunk_name: str,
    ) -> None:
        if function_index < 0 or function_index >= len(functions):
            raise CompileError(
                f"Verifier: invalid function index {function_index} at instruction {index} "
                f"in {chunk_name}"
            )
        if arg_count < 0:
            raise CompileError(
                f"Verifier: negative argument count at instruction {index} in {chunk_name}"
            )
        function = functions[function_index]
        if arg_count != function.param_count:
            raise CompileError(
                f"Function {function.name!r} expects {function.param_count} argument(s), "
                f"got {arg_count} at instruction {index} in {chunk_name}"
            )


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
                    and code[i + 2].op
                    in (Op.ADD, Op.SUB, Op.MUL, Op.DIV, Op.LT, Op.LTE, Op.GT, Op.GTE, Op.EQ, Op.NE)
                ):
                    a = code[i].arg
                    b = code[i + 1].arg
                    fold_op = code[i + 2].op

                    # Guard: preserve div-by-zero for runtime
                    if fold_op == Op.DIV and b == 0:
                        out.append(code[i])
                        i += 1
                        continue

                    if fold_op == Op.ADD:
                        result = a + b
                    elif fold_op == Op.SUB:
                        result = a - b
                    elif fold_op == Op.MUL:
                        result = a * b
                    elif fold_op == Op.DIV:  # b != 0 guaranteed above
                        result = a // b
                    elif fold_op == Op.LT:
                        result = 1 if a < b else 0
                    elif fold_op == Op.LTE:
                        result = 1 if a <= b else 0
                    elif fold_op == Op.GT:
                        result = 1 if a > b else 0
                    elif fold_op == Op.GTE:
                        result = 1 if a >= b else 0
                    elif fold_op == Op.EQ:
                        result = 1 if a == b else 0
                    else:  # NE
                        result = 1 if a != b else 0

                    out.append(Instr(Op.PUSH_INT, result))
                    i += 3
                    changed = True
                    continue

                out.append(code[i])
                i += 1
            code = out

        return tuple(code)


class TinyMemory:
    """
    Arena-backed integer slot storage.

    This is independent from the language-level Python dict model used by the
    original interpreter. Values are addressed by integer handles. The storage
    is still backed by Python objects because pure stdlib Python cannot bypass
    CPython's allocator safely.
    """

    __slots__ = ("_values", "_initialized")

    def __init__(self, slot_count: int) -> None:
        if slot_count < 0:
            raise ValueError("slot_count must be non-negative")
        self._values: list[int] = [0] * slot_count
        self._initialized: bytearray = bytearray(slot_count)

    def reset(self) -> None:
        self._values[:] = [0] * len(self._values)
        self._initialized[:] = b"\x00" * len(self._initialized)

    def load(self, slot: int) -> int:
        self._check_slot(slot)
        if self._initialized[slot] == 0:
            raise RuntimeTinyOneError(f"Read from uninitialized slot {slot}")
        return self._values[slot]

    def store(self, slot: int, value: int) -> None:
        self._check_slot(slot)
        if not isinstance(value, int):
            raise RuntimeTinyOneError(
                f"Memory accepts int values only, got {type(value).__name__}"
            )
        self._values[slot] = value
        self._initialized[slot] = 1

    def snapshot(self) -> tuple[int | None, ...]:
        return tuple(
            value if self._initialized[index] else None
            for index, value in enumerate(self._values)
        )

    def _check_slot(self, slot: int) -> None:
        if slot < 0 or slot >= len(self._values):
            raise RuntimeTinyOneError(f"Invalid memory slot {slot}")


def checked_div(lhs: int, rhs: int) -> int:
    if rhs == 0:
        raise RuntimeTinyOneError("Division by zero")
    return lhs // rhs


class VM:
    """Portable bytecode interpreter."""

    __slots__ = ("_program", "_memory", "_stdout")

    def __init__(self, program: Program, memory: TinyMemory, stdout: TextIO) -> None:
        self._program = program
        self._memory = memory
        self._stdout = stdout

    def run(self) -> None:
        self._run_chunk(self._program.code, self._memory, "main")

    def _run_chunk(
        self, code: tuple[Instr, ...], memory: TinyMemory, chunk_name: str
    ) -> int | None:
        stack: list[int] = []
        stdout = self._stdout
        pc = 0

        while True:
            instr = code[pc]
            pc += 1
            op = instr.op
            arg = instr.arg
            arg2 = instr.arg2

            if op == Op.PUSH_INT:
                stack.append(arg)
            elif op == Op.LOAD:
                stack.append(memory.load(arg))
            elif op == Op.STORE:
                memory.store(arg, stack.pop())
            elif op == Op.ADD:
                rhs = stack.pop()
                stack[-1] += rhs
            elif op == Op.SUB:
                rhs = stack.pop()
                stack[-1] -= rhs
            elif op == Op.MUL:
                rhs = stack.pop()
                stack[-1] *= rhs
            elif op == Op.DIV:
                rhs = stack.pop()
                stack[-1] = checked_div(stack[-1], rhs)
            elif op == Op.NEG:
                stack[-1] = -stack[-1]
            elif op == Op.LT:
                rhs = stack.pop()
                stack[-1] = 1 if stack[-1] < rhs else 0
            elif op == Op.LTE:
                rhs = stack.pop()
                stack[-1] = 1 if stack[-1] <= rhs else 0
            elif op == Op.GT:
                rhs = stack.pop()
                stack[-1] = 1 if stack[-1] > rhs else 0
            elif op == Op.GTE:
                rhs = stack.pop()
                stack[-1] = 1 if stack[-1] >= rhs else 0
            elif op == Op.EQ:
                rhs = stack.pop()
                stack[-1] = 1 if stack[-1] == rhs else 0
            elif op == Op.NE:
                rhs = stack.pop()
                stack[-1] = 1 if stack[-1] != rhs else 0
            elif op == Op.JUMP:
                pc = arg
            elif op == Op.JUMP_IF_ZERO:
                if stack.pop() == 0:
                    pc = arg
            elif op == Op.CALL:
                stack.append(self._call_function(arg, stack, arg2))
            elif op == Op.RETURN:
                return stack.pop()
            elif op == Op.PRINT:
                print(stack.pop(), file=stdout)
            elif op == Op.HALT:
                if stack:
                    raise RuntimeTinyOneError(
                        f"Internal stack imbalance at halt in {chunk_name}"
                    )
                return
            else:
                raise RuntimeTinyOneError(f"Unknown opcode {op!r}")

    def _call_function(self, function_index: int, caller_stack: list[int], arg_count: int) -> int:
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
        self._cache: dict[str, Callable[[TinyMemory, TextIO], None]] = {}

    def compile(self, program: Program) -> Callable[[TinyMemory, TextIO], None]:
        key = program.fingerprint
        cached = self._cache.get(key)
        if cached is not None:
            return cached

        source = self._build_source(program)
        namespace: dict[str, object] = {
            "checked_div": checked_div,
            "RuntimeTinyOneError": RuntimeTinyOneError,
            "TinyMemory": TinyMemory,
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
        lines = ["def _tinyone_jit(memory, stdout):"]
        sp = 0

        def slot(depth: int) -> str:
            return f"_s{depth}"

        for i, instr in enumerate(program.code):
            op = instr.op
            arg = instr.arg

            if op == Op.PUSH_INT:
                lines.append(f"    {slot(sp)} = {arg!r}")
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
                lines.append(f"    {slot(sp - 2)} = {slot(sp - 2)} + {slot(sp - 1)}")
                sp -= 1

            elif op == Op.SUB:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: SUB at index {i} requires depth>=2, got {sp} (verifier bug)"
                    )
                lines.append(f"    {slot(sp - 2)} = {slot(sp - 2)} - {slot(sp - 1)}")
                sp -= 1

            elif op == Op.MUL:
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: MUL at index {i} requires depth>=2, got {sp} (verifier bug)"
                    )
                lines.append(f"    {slot(sp - 2)} = {slot(sp - 2)} * {slot(sp - 1)}")
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
                lines.append(f"    {slot(sp - 1)} = -{slot(sp - 1)}")

            elif op in (Op.LT, Op.LTE, Op.GT, Op.GTE, Op.EQ, Op.NE):
                if sp < 2:
                    raise CompileError(
                        f"JIT codegen: {op.name} at index {i} requires depth>=2, got {sp} "
                        "(verifier bug)"
                    )
                operator = self._python_comparison_operator(op)
                lines.append(
                    f"    {slot(sp - 2)} = 1 if {slot(sp - 2)} {operator} {slot(sp - 1)} else 0"
                )
                sp -= 1

            elif op == Op.PRINT:
                if sp < 1:
                    raise CompileError(
                        f"JIT codegen: PRINT at index {i} with empty stack (verifier bug)"
                    )
                lines.append(f"    stdout.write(str({slot(sp - 1)}) + '\\n')")
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
            "def _tinyone_jit(memory, stdout):",
            "    return _tinyone_main(memory, stdout)",
            "",
            "def _tinyone_call(function_index, args, stdout):",
        ]
        if not program.functions:
            lines.append(
                "    raise RuntimeTinyOneError(f'Invalid function index {function_index}')"
            )
        else:
            for index, function in enumerate(program.functions):
                prefix = "if" if index == 0 else "elif"
                lines.append(f"    {prefix} function_index == {index}:")
                lines.append(f"        return _tinyone_func_{index}(args, stdout)")
            lines.append(
                "    raise RuntimeTinyOneError(f'Invalid function index {function_index}')"
            )
        lines.append("")

        for index, function in enumerate(program.functions):
            lines.extend(self._build_dispatch_function(index, function))
            lines.append("")

        lines.extend(
            self._build_dispatch_chunk(
                "_tinyone_main",
                program.code,
                program.slot_count,
                param_count=0,
                chunk_name="main",
                use_existing_memory=True,
            )
        )
        return "\n".join(lines) + "\n"

    def _build_dispatch_function(self, index: int, function: Function) -> list[str]:
        return self._build_dispatch_chunk(
            f"_tinyone_func_{index}",
            function.code,
            function.slot_count,
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
        param_count: int,
        chunk_name: str,
        use_existing_memory: bool,
    ) -> list[str]:
        if use_existing_memory:
            lines = [f"def {function_name}(memory, stdout):"]
        else:
            lines = [f"def {function_name}(args, stdout):"]
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
            lines.extend(self._build_dispatch_instr(instr, index))
        lines.append(
            f"        raise RuntimeTinyOneError(\"Invalid program counter in {chunk_name}\")"
        )
        return lines

    def _build_dispatch_instr(self, instr: Instr, index: int) -> list[str]:
        op = instr.op
        arg = instr.arg
        arg2 = instr.arg2
        next_pc = index + 1

        if op == Op.PUSH_INT:
            return [
                f"            stack.append({arg!r})",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.LOAD:
            return [
                f"            stack.append(memory.load({arg}))",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.STORE:
            return [
                f"            memory.store({arg}, stack.pop())",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.ADD:
            return [
                "            rhs = stack.pop()",
                "            stack[-1] = stack[-1] + rhs",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.SUB:
            return [
                "            rhs = stack.pop()",
                "            stack[-1] = stack[-1] - rhs",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.MUL:
            return [
                "            rhs = stack.pop()",
                "            stack[-1] = stack[-1] * rhs",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.DIV:
            return [
                "            rhs = stack.pop()",
                "            stack[-1] = checked_div(stack[-1], rhs)",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.NEG:
            return [
                "            stack[-1] = -stack[-1]",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op in (Op.LT, Op.LTE, Op.GT, Op.GTE, Op.EQ, Op.NE):
            operator = self._python_comparison_operator(op)
            return [
                "            rhs = stack.pop()",
                f"            stack[-1] = 1 if stack[-1] {operator} rhs else 0",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.JUMP:
            return [f"            pc = {arg}", "            continue"]
        if op == Op.JUMP_IF_ZERO:
            return [
                f"            pc = {arg} if stack.pop() == 0 else {next_pc}",
                "            continue",
            ]
        if op == Op.CALL:
            return [
                f"            args = [stack.pop() for _ in range({arg2})]",
                "            args.reverse()",
                f"            stack.append(_tinyone_call({arg}, args, stdout))",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.RETURN:
            return ["            return stack.pop()"]
        if op == Op.PRINT:
            return [
                "            stdout.write(str(stack.pop()) + '\\n')",
                f"            pc = {next_pc}",
                "            continue",
            ]
        if op == Op.HALT:
            return ["            return"]
        raise CompileError(f"JIT codegen: cannot emit unknown opcode {op!r}")

    @staticmethod
    def _python_comparison_operator(op: Op) -> str:
        if op == Op.LT:
            return "<"
        if op == Op.LTE:
            return "<="
        if op == Op.GT:
            return ">"
        if op == Op.GTE:
            return ">="
        if op == Op.EQ:
            return "=="
        if op == Op.NE:
            return "!="
        raise CompileError(f"JIT codegen: unsupported comparison opcode {op!r}")


def compile_source(source: str) -> Program:
    """
    Full compilation pipeline:
        Compiler  ->  PeepholeOptimizer  ->  BytecodeVerifier  ->  Program

    The returned Program is optimized and verified.  VM and JIT both receive
    the same bytecode, guaranteeing semantic equivalence.
    """
    program = Compiler(source).compile()
    program = PeepholeOptimizer.optimize(program)
    BytecodeVerifier.verify(program)
    LOGGER.debug(
        "compile_source complete",
        extra={"instructions": len(program.code), "slots": program.slot_count},
    )
    return program


def run_source(source: str, *, mode: str, stdout: TextIO) -> TinyMemory:
    program = compile_source(source)
    memory = TinyMemory(program.slot_count)

    if mode == "vm":
        VM(program, memory, stdout).run()
    elif mode == "jit":
        JitCache().compile(program)(memory, stdout)
    else:
        raise ValueError(f"Unsupported mode {mode!r}")

    return memory


def _configure_logging(verbose: bool) -> None:
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.WARNING,
        format="%(levelname)s %(name)s %(message)s",
    )


def parse_args(argv: Iterable[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="TinyOne compiler/VM/JIT")
    parser.add_argument("path", help="TinyOne source file")
    parser.add_argument(
        "--mode",
        choices=("jit", "vm"),
        default="jit",
        help="execution backend; jit compiles bytecode into Python locals",
    )
    parser.add_argument("--verbose", action="store_true", help="enable debug logging")
    return parser.parse_args(list(argv))


def main(argv: list[str]) -> int:
    args = parse_args(argv[1:])
    _configure_logging(args.verbose)

    try:
        with open(args.path, "r", encoding="utf-8") as file:
            source = file.read()
        run_source(source, mode=args.mode, stdout=sys.stdout)
        return 0
    except OSError as error:
        print(f"File error: {error}", file=sys.stderr)
        return 1
    except TinyOneError as error:
        print(f"TinyOne error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
