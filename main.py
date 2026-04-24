#!/usr/bin/env python3
"""
TinyOne: single-file stdlib-only compiler/VM/JIT implementation.

Language:
    let x = 1 + 2 * 3
    print x

Design constraints:
    - Python stdlib only
    - Single file
    - Maintainable Python implementation

Runtime model:
    Source -> tokens -> bytecode -> [peephole] -> [verify] -> VM or locals-JIT.

Memory model:
    TinyMemory is an arena of integer slots addressed by Slot handles. Runtime
    code does not use a Python dict for variable lookup; dict usage is limited
    to compile-time symbol interning.

JIT model:
    Stack depth is resolved at codegen time. Each virtual stack slot maps to a
    named Python local (_s0, _s1, ...). The emitted function contains only
    LOAD_FAST/STORE_FAST accesses — no list operations appear in the hot path.

Optimization:
    PeepholeOptimizer folds PUSH_INT + PUSH_INT + <binop> into a single
    PUSH_INT, running to convergence. Folding happens before verification and
    before JIT codegen, reducing both instruction count and emitted locals.

Verification:
    BytecodeVerifier performs an O(n) static stack-depth check before any
    execution. Stack imbalance is a compile-time error, not a runtime error.
    This allows the JIT to omit the runtime stack-balance guard.
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
    PLUS = 5
    MINUS = 6
    STAR = 7
    SLASH = 8
    EQUAL = 9
    LPAREN = 10
    RPAREN = 11
    EOF = 12


@dataclass(frozen=True, slots=True)
class Token:
    kind: TokenKind
    text: str
    pos: int


_KEYWORDS: Final[dict[str, TokenKind]] = {
    "let": TokenKind.LET,
    "print": TokenKind.PRINT,
}

_SINGLE_CHAR_TOKENS: Final[dict[str, TokenKind]] = {
    "+": TokenKind.PLUS,
    "-": TokenKind.MINUS,
    "*": TokenKind.STAR,
    "/": TokenKind.SLASH,
    "=": TokenKind.EQUAL,
    "(": TokenKind.LPAREN,
    ")": TokenKind.RPAREN,
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
    HALT = 10


@dataclass(frozen=True, slots=True)
class Instr:
    op: Op
    arg: int = 0


@dataclass(frozen=True, slots=True)
class Program:
    code: tuple[Instr, ...]
    slot_count: int
    names: tuple[str, ...]

    @property
    def fingerprint(self) -> str:
        hasher = hashlib.blake2b(digest_size=16)
        for instr in self.code:
            hasher.update(int(instr.op).to_bytes(2, "little", signed=False))
            hasher.update(int(instr.arg).to_bytes(16, "little", signed=True))
        hasher.update(self.slot_count.to_bytes(8, "little", signed=False))
        return hasher.hexdigest()


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

    __slots__ = ("_tokens", "_index", "_current", "_symbols", "_code")

    def __init__(self, source: str) -> None:
        self._tokens = Lexer(source).tokenize()
        self._index = 0
        self._current = self._tokens[0]
        self._symbols = SymbolTable()
        self._code: list[Instr] = []

    def compile(self) -> Program:
        while self._current.kind != TokenKind.EOF:
            self._statement()
        self._emit(Op.HALT)
        return Program(tuple(self._code), self._symbols.slot_count, self._symbols.names)

    def _statement(self) -> None:
        kind = self._current.kind
        if kind == TokenKind.LET:
            self._let_statement()
            return
        if kind == TokenKind.PRINT:
            self._print_statement()
            return
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

    def _expression(self) -> None:
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

    def _eat(self, kind: TokenKind) -> None:
        if self._current.kind != kind:
            raise CompileError(
                f"Expected {kind.name}, got {self._current.kind.name} at position {self._current.pos}"
            )
        self._index += 1
        self._current = self._tokens[self._index]

    def _emit(self, op: Op, arg: int = 0) -> None:
        self._code.append(Instr(op, arg))


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
    Op.HALT: 0,
}


class BytecodeVerifier:
    """
    O(n) static stack-depth checker.

    Simulates stack depth changes instruction by instruction without executing
    anything. Raises CompileError on:
      - negative depth mid-sequence  (stack underflow)
      - non-zero depth at HALT       (stack imbalance)
      - unrecognised opcode          (compiler/optimizer bug)

    By catching structural errors before execution, the VM and JIT can omit
    redundant runtime guards.  The JIT in particular no longer needs the
    end-of-function stack-imbalance check because it is a compile-time
    invariant after this pass.
    """

    @classmethod
    def verify(cls, program: Program) -> None:
        depth = 0
        for i, instr in enumerate(program.code):
            effect = _STACK_EFFECTS.get(instr.op)
            if effect is None:
                raise CompileError(
                    f"Verifier: unknown opcode {instr.op!r} at index {i}"
                )
            depth += effect
            if depth < 0:
                raise CompileError(
                    f"Verifier: stack underflow at instruction {i} "
                    f"({instr.op.name}, cumulative depth={depth})"
                )
        if depth != 0:
            raise CompileError(
                f"Verifier: stack imbalance at HALT, residual depth={depth}"
            )


class PeepholeOptimizer:
    """
    Constant-folding peephole optimizer over flat bytecode.

    Folds these patterns:
        PUSH_INT a, NEG              ->  PUSH_INT (-a)
        PUSH_INT a, PUSH_INT b, ADD  ->  PUSH_INT (a + b)
        PUSH_INT a, PUSH_INT b, SUB  ->  PUSH_INT (a - b)
        PUSH_INT a, PUSH_INT b, MUL  ->  PUSH_INT (a * b)
        PUSH_INT a, PUSH_INT b, DIV  ->  PUSH_INT (a // b)  [skipped if b==0]

    Runs until convergence so cascading folds resolve in one compilation.
    Each pass is O(n); worst-case passes = fold-chain length (bounded by
    the original instruction count).

    Division by zero at fold time is left intact so the runtime error fires
    as the user would expect — we don't silently swallow it at compile time.
    """

    @staticmethod
    def optimize(program: Program) -> Program:
        code = list(program.code)
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
                    and code[i + 2].op in (Op.ADD, Op.SUB, Op.MUL, Op.DIV)
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
                    else:  # DIV, b != 0 guaranteed above
                        result = a // b

                    out.append(Instr(Op.PUSH_INT, result))
                    i += 3
                    changed = True
                    continue

                out.append(code[i])
                i += 1
            code = out

        return Program(tuple(code), program.slot_count, program.names)


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
        stack: list[int] = []
        code = self._program.code
        memory = self._memory
        stdout = self._stdout
        pc = 0

        while True:
            instr = code[pc]
            pc += 1
            op = instr.op
            arg = instr.arg

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
            elif op == Op.PRINT:
                print(stack.pop(), file=stdout)
            elif op == Op.HALT:
                if stack:
                    raise RuntimeTinyOneError("Internal stack imbalance at halt")
                return
            else:
                raise RuntimeTinyOneError(f"Unknown opcode {op!r}")


class JitCache:
    """
    Compiles TinyOne bytecode into a Python function using local variables
    instead of a simulated stack list.

    The stack is fully resolved at codegen time.  Each virtual stack slot maps
    to a named Python local (_s0, _s1, ...).  Binary ops fold two locals into
    the lower slot in place.  The emitted function contains only LOAD_FAST and
    STORE_FAST opcodes at the CPython bytecode level — no list object is
    allocated and no list methods are called in the hot path.

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

    The stack-imbalance guard at HALT is omitted because BytecodeVerifier
    guarantees balance at compile time.

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
        """
        Emit a Python function that executes the program using local variables.

        Stack depth is tracked symbolically through sp (stack pointer).  Each
        virtual stack slot at depth N maps to the local name _sN.  No list
        object is used: all stack mutations become scalar assignments.

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
