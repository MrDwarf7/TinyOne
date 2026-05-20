# TinyOne Bytecode Reference

This document covers the TinyOne bytecode instruction set, the JSON artifact
format, the verifier's acceptance rules, and the JIT adaptive tier.

## Instruction Format

Every instruction is a fixed-size struct:

```rust
Instr { op: Op, arg: i64, arg2: i64 }
```

`arg` and `arg2` carry operands whose meaning is opcode-specific. Fields unused
by a particular opcode are zero. In the JSON artifact representation each
instruction is:

```json
{ "op": "OPCODE_NAME", "arg": 0, "arg2": 0 }
```

## Opcodes

There are 29 opcodes. Ordinals are stable — artifact round-trips depend on them
for fingerprinting and must not be changed.

| Ordinal | Mnemonic | `arg` | `arg2` | Stack effect | Notes |
| --- | --- | --- | --- | --- | --- |
| 1 | `PUSH_INT` | integer value | — | `→ int` | Push a literal integer |
| 2 | `LOAD` | slot index | — | `→ value` | Load from stack-frame slot |
| 3 | `STORE` | slot index | — | `value →` | Store to stack-frame slot |
| 4 | `ADD` | — | — | `a b → a+b` | Integer addition |
| 5 | `SUB` | — | — | `a b → a-b` | Integer subtraction |
| 6 | `MUL` | — | — | `a b → a*b` | Integer multiplication |
| 7 | `DIV` | — | — | `a b → a//b` | Floor division; runtime error on divide-by-zero |
| 8 | `NEG` | — | — | `a → -a` | Unary integer negation |
| 9 | `PRINT` | — | — | `value →` | Print value to stdout, with newline |
| 10 | `LT` | — | — | `a b → 0\|1` | Less-than comparison |
| 11 | `LTE` | — | — | `a b → 0\|1` | Less-than-or-equal |
| 12 | `GT` | — | — | `a b → 0\|1` | Greater-than |
| 13 | `GTE` | — | — | `a b → 0\|1` | Greater-than-or-equal |
| 14 | `EQ` | — | — | `a b → 0\|1` | Equality (integers only) |
| 15 | `NE` | — | — | `a b → 0\|1` | Not-equal (integers only) |
| 16 | `JUMP` | target pc | — | — | Unconditional jump to `arg` |
| 17 | `JUMP_IF_ZERO` | target pc | — | `cond →` | Jump to `arg` if top is zero; pop condition |
| 18 | `CALL` | function index | arg count | `args… → ret` | Call function `arg`; `arg2` args popped from stack |
| 19 | `RETURN` | — | — | `value →` | Return top of stack from current function |
| 20 | `HALT` | — | — | — | Terminate the main chunk |
| 21 | `PUSH_STRING` | string table index | — | `→ string` | Push string heap reference |
| 22 | `MAKE_ARRAY` | element count | — | `elems… → array` | Pop `arg` values; construct heap array |
| 23 | `INDEX` | — | — | `arr idx → value` | Array or string index |
| 24 | `SET_INDEX` | — | — | `arr idx value →` | Mutate array element |
| 25 | `MAKE_STRUCT` | struct def index | field count | `fields… → struct` | Construct heap struct |
| 26 | `GET_FIELD` | field table index | — | `struct → value` | Read named field |
| 27 | `SET_FIELD` | field table index | — | `struct value →` | Write named field |
| 28 | `BUILTIN` | builtin table index | arg count | `args… → result` | Call builtin or stdlib bridge |
| 29 | `PUSH_NULL` | — | — | `→ null` | Push the null raw pointer |

### Notes on specific opcodes

**`CALL`** — `arg` is a zero-based index into `program.functions`. The callee's
frame is initialized with the top `arg2` values popped from the caller's stack.
Additional slots are zero-initialized. The return value is pushed after the call
returns.

**`BUILTIN`** — Phase-1 builtins occupy slots 0–34 in the canonical table.
Phase-2 stdlib bridge builtins follow after slot 34. Slot order within each
group is stable and must not be reordered without a bytecode version bump.

**`MAKE_STRUCT`** — `arg` indexes into `program.structs` for the field names;
`arg2` is the number of fields (and values to pop). The compiler always sets
`arg2 == structs[arg].fields.len()`.

**`GET_FIELD` / `SET_FIELD`** — `arg` is an index into `program.fields`, the
global field-name intern table shared across all structs.

**`JUMP` / `JUMP_IF_ZERO`** — targets are absolute instruction indices, not
relative offsets. The verifier bounds-checks all targets before execution.

## Program Structure

A `Program` contains:

```
Program {
  code:       Vec<Instr>        — main chunk
  slot_count: usize             — main frame slot count
  names:      Vec<String>       — variable name table (debug only)
  functions:  Vec<Function>     — named function chunks
  strings:    Vec<String>       — string literal intern table
  structs:    Vec<StructDef>    — struct definitions (name + field names)
  fields:     Vec<String>       — global field-name intern table
  modules:    Vec<ModuleDef>    — import/export metadata
}

Function {
  name:        String
  param_count: usize
  code:        Vec<Instr>
  slot_count:  usize
  names:       Vec<String>
}
```

Functions are indexed from zero. The main chunk is not in `functions`; it is
`program.code`. Module metadata records the original import strings and export
lists; it does not affect runtime behavior.

## JSON Artifact Format

```json
{
  "format":     "tinyone-bytecode",
  "version":    1,
  "code":       [ {"op": "PUSH_INT", "arg": 42, "arg2": 0}, … ],
  "slot_count": 0,
  "names":      [],
  "functions":  [
    {
      "name":        "add",
      "param_count": 2,
      "code":        [ … ],
      "slot_count":  2,
      "names":       ["a", "b"]
    }
  ],
  "strings":    ["hello", "world"],
  "structs":    [ {"name": "Point", "fields": ["x", "y"]} ],
  "fields":     ["x", "y"],
  "modules":    [
    {
      "name":               "math",
      "path":               "math.to",
      "imports":            [],
      "exported_functions": ["add"],
      "exported_structs":   []
    }
  ]
}
```

### Artifact resource limits

These are checked by `Program::from_artifact` before any `Vec::collect`:

| Field | Limit |
| --- | --- |
| Total artifact bytes | 8 MiB |
| `functions` count | 4,096 |
| `structs` count | 4,096 |
| `code` ops per chunk | 65,536 |
| Total ops across all chunks | 262,144 |
| `strings` count | 65,536 |
| `fields` count | 65,536 |
| `slot_count` (per chunk) | 65,536 |
| `modules` count | 256 |
| `imports` per module | 4,096 |
| `exports` per module | 4,096 |
| Fields per struct | 256 |
| `names` count | 65,536 |
| Total string/name/field text bytes | 1 MiB |

A hostile artifact that exceeds any limit receives a structured `TinyOneError`
before any allocation for the program body.

## Program Fingerprint

`Program::fingerprint()` hashes the complete program with Blake2b512 and
returns the first 16 bytes as a lowercase hex string. The hash covers opcodes,
operands, slot counts, function names and arities, string literals, struct
definitions, field names, and module metadata. It is used as the cache key for
`JitCache` and may be used for artifact integrity checks.

## Verifier Rules

`BytecodeVerifier::verify(program)` accepts a program if and only if all of the
following hold for every chunk (main and all functions):

1. **Terminal opcode** — the chunk's last reachable instruction is `HALT` (main)
   or `RETURN` (function).
2. **Stack balance** — at every instruction, the operand stack depth inferred by
   BFS is non-negative and consistent across all paths reaching that instruction.
3. **Stack depth** — stack depth never exceeds 65,536.
4. **Branch targets** — all `JUMP` and `JUMP_IF_ZERO` targets are within
   `[0, code.len())`.
5. **Slot indexes** — all `LOAD` and `STORE` operands are in
   `[0, chunk.slot_count)`.
6. **String indexes** — all `PUSH_STRING` operands are in
   `[0, program.strings.len())`.
7. **Field indexes** — all `GET_FIELD` and `SET_FIELD` operands are in
   `[0, program.fields.len())`.
8. **Struct indexes** — all `MAKE_STRUCT` operands are in
   `[0, program.structs.len())`.
9. **Function indexes** — all `CALL` operands are in
   `[0, program.functions.len())`, and the arity matches `function.param_count`.
10. **Builtin indexes** — all `BUILTIN` operands are in
    `[0, BUILTINS.len())`, and the argument count is within the builtin's
    declared range `[min_args, max_args]`.
11. **Budget limits** — total ops across all chunks ≤ 262,144; function count ≤
    4,096; per-chunk slot count ≤ 65,536; string/field/name count ≤ 65,536;
    struct count ≤ 4,096; module count ≤ 256; imports/exports per module ≤ 4,096;
    total text bytes ≤ 1 MiB.
12. **Work limit** — BFS step count ≤ 10,000,000. Prevents timeout on adversarial
    jump graphs.

## JIT Adaptive Tier

### Compilation

`JitProgram::compile` translates verified TinyOne bytecode into `JitOp` — a
Rust enum with all operands decoded to native types at compile time:

- `LOAD 3` → `JitOp::Load(3usize)` — no conversion at runtime
- `PUSH_INT 42` → `JitOp::PushInt(42i64)`
- `CALL 1 2` → `JitOp::Call(1usize, 2usize)`

Superinstructions fuse common two- and three-instruction sequences:

| Bytecode sequence | JIT superinstruction |
| --- | --- |
| `PUSH_INT n, STORE s` | `StoreInt(s, n)` |
| `LOAD s, PUSH_INT n, ADD, STORE s` | `AddSlotInt(s, n)` |
| `LOAD s, PUSH_INT n, SUB, STORE s` | `SubSlotInt(s, n)` |

### Hot-path quickening

Every backward branch in a compiled chunk increments an edge counter. When a
counter reaches 8 (`HOT_BACK_EDGE_THRESHOLD`), the range
`[branch_target, branch_instruction + 1)` is promoted in-place to faster
variants:

| Cold | Quickened | Effect |
| --- | --- | --- |
| `Add` | `AddInt` | Skips `Value` tag check; asserts integer |
| `Sub` | `SubInt` | Same |
| `Mul` | `MulInt` | Same |
| `Div` | `DivInt` | Same |
| `Compare(op)` | `CompareInt(op)` | Same |
| `Jump(t)` | `JumpHot(t)` | Skips back-edge counter increment |
| `JumpIfZero(t)` | `JumpIfZeroHot(t)` | Same |

Quickening is in-place and permanent for the lifetime of the `JitProgram`.
Programs with heterogeneous loop payloads (integers mixed with heap values) will
receive the quickened jump ops even if the arithmetic ops do not qualify.

### Cache

`JitCache` stores `JitProgram` instances keyed by fingerprint in a
`HashMap<String, JitProgram>`. A cache hit reuses the compiled and potentially
already-quickened program, so hot ranges from a previous run carry over.

`JitCache::compile` re-verifies the program before inserting it. The public
`run_*` methods each verify once then delegate to internal `*_unchecked` helpers
that skip re-verification.

## Peephole Optimizer

The optimizer runs a single forward pass before verification. It folds
constant-only sequences within straight-line (branch-free) code:

| Input sequence | Output |
| --- | --- |
| `PUSH_INT a, PUSH_INT b, ADD` | `PUSH_INT (a+b)` |
| `PUSH_INT a, PUSH_INT b, SUB` | `PUSH_INT (a-b)` |
| `PUSH_INT a, PUSH_INT b, MUL` | `PUSH_INT (a*b)` |
| `PUSH_INT a, PUSH_INT b, DIV` | `PUSH_INT (a//b)` (if b≠0) |
| `PUSH_INT a, NEG` | `PUSH_INT (-a)` |
| `PUSH_INT a, PUSH_INT b, LT/LTE/GT/GTE/EQ/NE` | `PUSH_INT (0 or 1)` |

Folding stops at any branch or non-constant opcode. The optimizer does not
reorder or eliminate instructions and does not analyze across basic block
boundaries.
