# TinyOne Implementation Architecture

This document describes the current implementation, not a permanent
architecture promise. TinyLang v1 keeps its ABI frozen, but the allocator, VM,
JIT, memory design, syntax, and semantics may evolve. TinyLang v2 is expected
to make major changes as the language boundary and implementation model are
redesigned. Send comments, concerns, and questions to the [TinyLang community
forum](https://tl.404connernotfound.dev).

This document describes how the TinyOne implementation is structured
internally: the pipeline each TinyLang program travels through, the major
modules, and the invariants each stage owns.

## Pipeline Overview

```
source text
    │
    ▼
┌─────────┐
│  Lexer  │  syntax/lexer.rs, syntax/token.rs
└────┬────┘
     │  Vec<Token>
     ▼
┌──────────┐
│ Compiler │  compiler/mod.rs, compiler/parser.rs,
│          │  compiler/state.rs, compiler/symbols.rs,
│          │  compiler/modules.rs
└────┬─────┘
     │  Program (unoptimized)
     ▼
┌───────────┐
│ Optimizer │  bytecode/peephole.rs
└────┬──────┘
     │  Program (optimized)
     ▼
┌──────────┐
│ Verifier │  bytecode/verifier.rs
└────┬─────┘
     │  (error or proceeds)
     ▼
  ┌──┴──┐
  │     │
  ▼     ▼
 VM    JIT
```

The public API surface (`api.rs`) ties these stages together. The
`compile_*_verified` entry points return a `VerifiedProgram` capability that
keeps both the immutable `Program` and its memoized fingerprint. Compatibility
entry points still return `Arc<Program>`. Execution APIs accepting the verified
capability skip duplicate verification and fingerprint work.

File compilation may first consult the dependency-validated disk cache. A
valid compact binary artifact bypasses lexing, parsing, optimization, and
module assembly, but is still decoded under resource limits and verified.

## Module Map

```
TinyOne/src/
├── lib.rs              Public re-exports; feature gates for testing
├── api.rs              compile_source, compile_file, lex_source, optimize_program
├── cli.rs              CLI argument parsing and dispatch
├── runner.rs           run_program, run_program_report, run_program_with_env,
│                       run_source, run_source_report
├── error.rs            TinyOneError (Compile | Runtime), Result<T>
├── source.rs           SourceMap — filename tracking for diagnostics
├── builtins.rs         BUILTINS table, builtin_index lookup
├── artifact_io.rs      load_artifact, write_artifact (file-level I/O)
├── ffi.rs              extern "C" entry points and JSON response helpers
│
├── syntax/
│   ├── mod.rs
│   ├── lexer.rs        Lexer: source text → Vec<Token>
│   └── token.rs        TokenKind enum
│
├── bytecode/
│   ├── mod.rs
│   ├── opcode.rs       Op enum (31 opcodes), name/from_name/ordinal
│   ├── instr.rs        Instr { op, arg: i64, arg2: i64 }
│   ├── program.rs      Program, Function, StructDef, ModuleDef,
│   │                   VerifiedProgram, fingerprint (Blake2b512)
│   ├── artifact.rs     Program::to_artifact / from_artifact (JSON serde),
│   │                   resource limits, reject_over_limit
│   ├── peephole.rs     PeepholeOptimizer — constant folding pass
│   └── verifier.rs     BytecodeVerifier — BFS stack-depth + control-flow check
│
├── compiler/
│   ├── mod.rs          Compiler entry point, module resolution wiring
│   ├── parser.rs       Recursive-descent parser + bytecode emitter
│   ├── state.rs        CompilerState, CompilerSharedState
│   ├── symbols.rs      SymbolTable — scoped slot allocation
│   └── modules.rs      Module import/export resolution, resolve_import
│
├── runtime/
│   ├── mod.rs          Public runtime re-exports
│   ├── vm.rs           VM interpreter, TinyRunReport
│   ├── heap.rs         TinyHeap, HeapObject, HeapData, TinyHeapStats
│   ├── memory.rs       TinyMemory — stack-frame slot vector
│   ├── context.rs      TinyRuntimeContext — heap + I/O + sys args/env
│   ├── value.rs        Value (Int | Heap | Pointer), RuntimeValue
│   ├── aggregate.rs    Array/struct/buffer/cell runtime operations
│   ├── arithmetic.rs   runtime_add, runtime_sub, runtime_mul, runtime_div
│   ├── builtins.rs     runtime_call_builtin dispatcher
│   ├── stdlib.rs       Phase-2 stdlib bridge (vec, map, io, string, …)
│   ├── pointers.rs     Raw-pointer operations (ptr_load, ptr_store, …)
│   ├── format.rs       runtime_print formatting
│   ├── limits.rs       Runtime resource constants
│   └── typing.rs       TypeKind enum, type_of, smallest_fit, promote
│
└── jit/
    ├── mod.rs
    ├── op.rs           JitOp enum — decoded, operand-unboxed instructions
    ├── chunk.rs        JitChunk — compiled function/main chunk
    ├── program.rs      JitProgram — full compiled program + hot-path quickening
    ├── cache.rs        JitCache — fingerprint-keyed program cache
    └── vm.rs           JitVm — JitOp interpreter
```

## Stage Details

### Lexer

`Lexer::new(source, filename).tokenize()` scans the source in one pass and
returns a flat `Vec<Token>`. Tokens carry their `TokenKind` and a source
position for error messages. The lexer rejects non-ASCII characters that are
not part of a string literal.

### Compiler

`Compiler::new(source, filename, resolver, ...)` constructs a single-pass
recursive-descent compiler. Parsing and bytecode emission happen in the same
pass — there is no AST. `compile()` returns an unoptimized `Program`.

Key compiler subsystems:

- **SymbolTable** — lexically scoped slot allocator. Each `let` declaration
  claims the next stack slot in the current scope. Block exit does not reclaim
  slots; slots are zero-initialized at frame entry and hidden after their scope
  exits.
- **Module resolution** — all nested compilers share one `ModuleResolver`.
  Canonical paths, manifest probes (including missing manifests), parsed
  manifests, import results, and source text are memoized for the compilation
  session. A source file is therefore read once even when reached through
  several aliases. Circular imports are detected in `CompilerSharedState`.

### Compile Cache

The CLI enables a sibling `.tinyone-cache/` by default. Its metadata records
the compiler/cache format version, optimization mode, root path, content
digests for every source and manifest probe, canonical path resolutions, and
logical module names. A hit loads the compact `.tob` artifact and verifies its
fingerprint before use. Missing, stale, malformed, or unverifiable entries are
ordinary misses.

When exactly one existing module source changed, TinyOne attempts a bounded
incremental rebuild. It recompiles that module and its imports, relocates table
indexes by semantic name/content, replaces only declarations owned by that
module, and verifies the combined program. Declaration-topology changes or any
failed invariant fall back to a full compile. Cache failures never prevent a
correct source build.

### Optimizer

`PeepholeOptimizer::optimize(program)` runs bounded forward passes over each
chunk's instruction stream. It folds adjacent constant pushes through
arithmetic and comparison opcodes, collapsing patterns like
`PUSH_INT 2, PUSH_INT 3, MUL` into `PUSH_INT 6`. It optimizes straight-line
regions inside control-flow-heavy chunks, never consumes an interior branch
target, and remaps jump offsets after each shrinking pass.

### Verifier

`BytecodeVerifier::verify(program)` runs a bounded BFS over the control-flow
graph of every chunk. It tracks the operand-stack depth at each reachable
instruction and rejects programs where:

- Stack depth is inconsistent at a join point (a target reached from two paths
  with different depths).
- Stack depth exceeds `MAX_STACK_DEPTH` (65,536).
- A branch target is out of range.
- A slot index, string index, field index, or struct index is out of range.
- Builtin argument count is outside the declared `[min_args, max_args]` range.
- Module metadata is inconsistent, forged, cyclic, or names a missing export.
- A call, function value, struct construction, enum construction, or global
  load crosses a module boundary without the required export/import authority.
- A chunk does not end with the required terminal opcode (`HALT` for main,
  `RETURN` for functions).
- Work steps exceed `MAX_VERIFIER_STEPS` (10,000,000) — guards against
  adversarially crafted dense control-flow graphs.

The BFS uses a `seen: HashMap<pc → stack_depth>` to break backward-edge loops:
if a target PC is visited again with the same stack depth, it is not re-queued;
if visited with a different depth it is immediately rejected as a stack
mismatch.

All verification runs before any allocation for execution.

The VM and JIT share this verifier. Dynamic function-name entry points
(`closure_new` and `thread_spawn`) separately restrict lookups to root or
exported module functions because their target is supplied as runtime data,
not encoded in an instruction. Native shared-library imports currently fail
closed: arbitrary DLL/SO execution is outside the verified bytecode boundary
until a versioned native ABI and isolation policy are implemented.

### VM Backend

`VM::new(program, memory, inputs)` verifies the program and constructs an
interpreter. `vm.run(stdout)` enters the main instruction loop. Each opcode
maps directly to a Rust operation against the operand stack, the stack-frame
memory, and the heap context.

Function calls push a new frame onto a call-depth counter (limit: 16) and
dispatch to the function's bytecode chunk. All VM operations return `Result`;
there are no `unwrap` or `panic` calls on production paths.

`VM::new_unchecked(program, memory, inputs)` skips re-verification and is
used internally by `runner.rs` which has already verified the program.

### JIT Backend

The JIT is an **adaptive bytecode tier**, not a native machine-code JIT.

**Compilation** (`JitProgram::compile`):
1. Accepts or constructs a `VerifiedProgram` (same verifier as the VM path).
2. Translates the main `Instr { op, arg, arg2 }` chunk into a `JitOp` — a Rust enum
   variant with decoded, type-safe operands already converted to `usize` or
   `i64`. No operand decoding happens at runtime.
3. Builds `store.i` (`StoreInt`) and `slot.add.i` / `slot.sub.i` (`AddSlotInt`
   / `SubSlotInt`) superinstructions for common `PUSH_INT, STORE` and
   `LOAD, PUSH_INT, ADD/SUB, STORE` sequences.

Function chunks are lowered into reserved slots only on their first call.
`JitProgram` retains the verified program as the single owner of strings,
functions, structs, fields, modules, and enum metadata instead of cloning
those tables. Human-readable listing generation explicitly lowers all chunks.

**Hot-loop quickening**:
Each compiled chunk carries an `edge_counts: Vec<u16>` parallel to its ops.
`JitVm` increments the counter at every backward branch. When a counter reaches
the configured threshold (8 by default), `JitChunk::promote_range(target, end)` rewrites
all ops in `[target, end)` that have a faster "hot" variant:

| Cold op | Hot op |
| --- | --- |
| `Add` | `AddInt` |
| `Sub` | `SubInt` |
| `Mul` | `MulInt` |
| `Div` | `DivInt` |
| `Compare(op)` | `CompareInt(op)` |
| `Jump(target)` | `JumpHot(target)` |
| `JumpIfZero(t)` | `JumpIfZeroHot(t)` |

`*Int` variants skip the `Value` tag check and operate directly on the inner
`i64`. `*Hot` variants skip the branch-counter increment. Quickening is
in-place and irreversible within a run.

`JitOptions` configures this threshold for `JitProgram`, `JitCache`, and the
configured runner APIs. Threshold zero disables quickening while retaining JIT
lowering and superinstructions.

**Caching** (`JitCache`):
Programs are identified by their Blake2b512 fingerprint (truncated to 16 hex
bytes). A `HashMap<String, JitProgram>` stores compiled programs. On a cache
hit, quickened state from a previous run is preserved across repeated calls.

Runtime maps retain insertion-ordered Ralloc entries and maintain a canonical
key index for average constant-time `map_get`, `map_has`, and `map_set`
lookup. Integer widths normalize by value, strings index by content, and heap
or pointer identities include their allocation generation. Pointer-key access
still validates every stored pointer key before consulting the index.

### Heap

`TinyHeap` is a generational slab. Slots hold `Option<HeapObject>` and a
parallel `generations: Vec<u64>` counter. When an object is allocated, it
claims the next free slot (or appends), and the generation is incremented.
When freed, the slot is set to `None` and added to `free`.

Every `HeapRef { address, generation }` carries the generation at allocation
time. Before any access, the runtime checks `stored_generation == current` and
returns `TinyOneError::runtime("Stale heap reference …")` if they differ. This
catches use-after-free and prevents a new allocation at the same address from
being confused with the old object.

`RawPointer { address, kind, index, field, generation, cast }` derives from a
`HeapRef` and adds a kind tag (`"null"`, `"object"`, `"array"`, `"buffer"`,
`"field"`) plus an element index or field name. Before raw-pointer use, the
runtime validates the base heap reference (generation check), the kind, and the
index or offset bounds.

### FFI Layer

`ffi.rs` exposes `extern "C"` entry points that all funnel through `respond()`.
`respond` wraps `response_cstring` in `catch_unwind(AssertUnwindSafe(...))`.
`response_cstring` wraps the actual callback in a second `catch_unwind`. If
either unwind guard fires, the fallback is a static byte literal that requires
no allocation. All JSON responses follow `{"ok": true/false, "kind": "…",
"value"/"error": …}`. See `docs/ffi/c-integration.md` for the full contract.

## Key Invariants

1. **Verify before execute.** Every public execution path calls
   `BytecodeVerifier::verify` exactly once before any instruction is run.
   Internal paths that chain from an already-verified call use `*_unchecked`
   constructors to avoid redundant re-verification.

2. **No panic on production paths.** All operations that can fail return
   `Result`. The only `unwrap`/`expect` calls in non-test code are inside
   `catch_unwind` guards in `ffi.rs` or in benchmark binaries.

3. **Limits before allocation.** Resource limits (artifact sizes, code counts,
   heap bytes, buffer sizes) are checked before `Vec::collect`, `File::open`,
   or heap allocation. Hostile inputs fail with a structured error rather than
   triggering unbounded allocation.

4. **Generation tags prevent dangling references.** `HeapRef` and `RawPointer`
   both carry a generation counter. A freed-then-reallocated slot has a
   different generation, so stale references are caught at the next access.

5. **ABI version 1 is frozen.** The `extern "C"` entry points, their JSON
   schemas, and the `tinylang.h` header remain frozen for the v1 lifecycle.
