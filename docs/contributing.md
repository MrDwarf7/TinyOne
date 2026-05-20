# Contributing to TinyOne

## Prerequisites

- Rust toolchain with Cargo (stable, edition 2024)
- A C compiler (`cc`) for the FFI smoke test
- Python 3 for `Tools/hash.py` (optional; used for release manifests)

## Building

```sh
# Build the CLI and library
cargo build --manifest-path Rust/Cargo.toml

# Build the release binary
cargo build --release --manifest-path Rust/Cargo.toml
```

## Running Tests

```sh
# Standard test suite (101 tests, all should pass)
cargo test --manifest-path Rust/Cargo.toml

# Language fixture suite (requires testing-hooks feature)
cargo test --manifest-path Rust/Cargo.toml --features testing-hooks

# Single test suite
cargo test --manifest-path Rust/Cargo.toml --test runtime_parity
cargo test --manifest-path Rust/Cargo.toml --test abi_api_soundness
cargo test --manifest-path Rust/Cargo.toml --test stdlib_parity

# Run the C FFI smoke test (requires pre-built cdylib)
cargo build --manifest-path Rust/Cargo.toml
cargo test --manifest-path Rust/Cargo.toml --test abi_api_soundness \
  c_header_ffi_smoke_covers_ownership_null_and_mode_contracts
```

## Code Quality

```sh
# Format check (must pass before commit)
cargo fmt --manifest-path Rust/Cargo.toml --all --check

# Apply formatting
cargo fmt --manifest-path Rust/Cargo.toml --all

# Lints (must be clean)
cargo clippy --manifest-path Rust/Cargo.toml --all-targets -- -D warnings
```

## Benchmarks

```sh
# Full benchmark suite
cargo run --release --manifest-path Rust/Cargo.toml --bin tinyone-bench

# Quick smoke run
cargo run --release --manifest-path Rust/Cargo.toml --bin tinyone-bench \
  -- --quick --repeats 1
```

## Source-Tree Integrity Check

```sh
./Tools/hash.py --tree . \
  --include .py --include .rs --include .toml --include .md \
  --format json
```

---

## Adding a Language Feature

Most language changes touch the compiler, possibly the verifier, and possibly
the VM/JIT. Follow this sequence:

### 1. Add a test fixture first

Add a `.to` file to `Rust/tests/Language/pass/`, `fail_compile/`, or
`fail_runtime/` depending on expected behavior. Run the language suite to
confirm the fixture fails before your change:

```sh
cargo test --manifest-path Rust/Cargo.toml --features testing-hooks 2>&1 | grep FAIL
```

### 2. Extend the lexer if needed (`syntax/lexer.rs`, `syntax/token.rs`)

New keywords become new `TokenKind` variants. The lexer is a manual single-pass
scanner; add a branch to the character-dispatch loop.

### 3. Extend the compiler (`compiler/parser.rs`)

The parser is recursive descent. Each statement or expression form has its own
`parse_*` method that emits bytecode directly. Add a branch to the appropriate
`parse_statement` or `parse_expression` dispatcher.

### 4. Add new opcodes if needed (`bytecode/opcode.rs`)

Append new `Op` variants to the enum. Add them to `name()`, `from_name()`, and
`ordinal()`. **Ordinals must be stable once assigned.** Assign the next
sequential value; do not reuse a retired ordinal. Update the verifier's
stack-effect table and the VM and JIT dispatch loops.

### 5. Update the verifier (`bytecode/verifier.rs`)

The verifier's `verify_chunk` BFS tracks stack depth. Add the stack effect of
any new opcode to the match arm inside `verify_chunk`. Also add any new operand
range checks (slot indexes, string indexes, etc.).

### 6. Update the VM (`runtime/vm.rs`)

Add a match arm in `run_chunk` for the new opcode. Operations that can fail must
return `Result`.

### 7. Update the JIT (`jit/op.rs`, `jit/chunk.rs`)

- In `JitOp::from_instr`: translate the new `Op` to a `JitOp` variant.
- In `JitOp::listing`: add a display string.
- In `JitChunk::compile` (if applicable): add any superinstruction patterns.
- In `JitVm` (`jit/vm.rs`): add the execution branch.

### 8. Update the artifact format if needed (`bytecode/artifact.rs`)

If the new opcode uses operands stored in the program's string or field tables,
ensure `to_artifact` serializes them and `from_artifact` deserializes and
validates them under the existing limits.

### 9. Run the full suite

```sh
cargo test --manifest-path Rust/Cargo.toml
cargo test --manifest-path Rust/Cargo.toml --features testing-hooks
cargo clippy --manifest-path Rust/Cargo.toml --all-targets -- -D warnings
```

---

## Adding a Builtin Function

### Phase-1 (core) builtin

Phase-1 builtins occupy the first 35 slots of `BUILTINS`. These are frozen —
do not insert into or reorder them.

To add a Phase-2 stdlib bridge builtin:

1. **`builtins.rs`** — append a new `Builtin { name, arg_count }` entry after
   index 34 in `BUILTINS`. The name must not conflict with any existing builtin.
2. **`runtime/builtins.rs`** — add a dispatch arm in
   `runtime_call_stdlib_builtin` that calls your new function.
3. **`runtime/stdlib.rs`** — implement the function as `pub fn b_your_name(…)`
   following the existing pattern. Return `Result<Value>`. Do not panic.
4. **`stdlib/`** — optionally add a TinyOne-language wrapper in the appropriate
   `stdlib/*.to` module and export it from the module's `tinyone.json` manifest.
5. **`Rust/tests/stdlib_parity.rs`** — add at least one test covering the new
   builtin via `run_source`.

### Rules for all builtins

- Never use `.unwrap()` or `.expect()` in a builtin implementation.
- Check argument types explicitly; return `TinyOneError::runtime(…)` on type
  mismatch rather than panicking.
- Respect heap limits (MAX_HEAP_BYTES, MAX_ARRAY_LENGTH, MAX_BUFFER_BYTES).
  Validate sizes before allocating.
- If the builtin touches the filesystem, it must require `unsafe` at the call
  site and must check size or count limits before reading.

---

## Adding Stdlib Modules

Stdlib modules are TinyOne source files in `stdlib/` registered in
`stdlib/tinyone.json`. They wrap Phase-2 bridge builtins for ergonomic use.

1. Create `stdlib/your_module.to`.
2. Export functions with `export fn name(…)`.
3. Add `"your_module": "your_module.to"` to `stdlib/tinyone.json`.
4. Users can then `import "your_module" as m` with the stdlib manifest in scope.

---

## Adding Test Fixtures

### Language fixture (`.to` file)

Place fixtures under `Rust/tests/Language/`:

- `pass/` — programs expected to compile and print deterministic output.
- `fail_compile/` — programs expected to fail at compile time.
- `fail_runtime/` — programs expected to compile but fail at runtime.

The language suite reads fixture files and checks their outcomes automatically
when run with `--features testing-hooks`.

### Integration test (Rust)

Add a `#[test]` function to the appropriate file:

- `Rust/tests/runtime_parity.rs` — language behavior, VM/JIT parity
- `Rust/tests/stdlib_parity.rs` — stdlib behavior
- `Rust/tests/abi_api_soundness.rs` — FFI contracts, artifact limits, verifier

All test functions must:
- Use `run_source` or `compile_source` rather than constructing `Program` by hand
  unless specifically testing the `Program` struct.
- Assert both `vm` and `jit` modes agree when testing language behavior.
- Return nothing (`fn test_name()`) and use `assert!` / `assert_eq!` rather
  than returning `Result`.

---

## Error Handling Rules

- Production code must never call `.unwrap()`, `.expect()`, `panic!()`, or
  `unreachable!()` outside of `catch_unwind` guards or `#[cfg(test)]` blocks.
- All operations that can fail must return `Result<T>` and propagate errors
  with `?`.
- Errors must be `TinyOneError::compile(…)` for compilation-time failures or
  `TinyOneError::runtime(…)` for execution-time failures.
- Resource limits must be checked **before** any allocation. Do not allocate a
  `Vec` from an untrusted count field before validating that count against its
  limit.

---

## FFI Entry Points

When adding a new `extern "C"` function:

1. Add the declaration to `tinyone.h` with a `# Safety` doc comment.
2. Add the Rust implementation in `ffi.rs`.
3. If the function returns `char *`, it **must** route through `respond()` to
   get the double-`catch_unwind` panic boundary.
4. If the function is `void`-returning, it **must** install its own
   `catch_unwind` guard — `respond()` cannot be used for void functions.
5. Nullable `const char *` parameters must be annotated `/* nullable */` in the
   header.
6. Add a test in `Rust/tests/abi_api_soundness.rs` exercising the null-pointer
   and error cases.

---

## Commit Message Convention

```
area(scope): short imperative description

Longer explanation if needed. Reference specific file:line for non-obvious
decisions.
```

Examples:
```
feat(compiler): add else-if chain support
fix(jit): eliminate double-verification on cached run paths
docs(stdlib): document map_del return value
test(abi): add adversarial test for empty artifact code array
```
