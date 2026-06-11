# TinyOne v1 Roadmap

Target release date: **August 1, 2026**

This document describes the work required before v1 is tagged and the ABI is
declared stable. Items are grouped by area and ordered by blocking priority:
items earlier in each section must be resolved before items that follow them.

---

## Blocking: ABI Stability

The C FFI ABI is explicitly marked **UNSTABLE** until all items in this section
are resolved. Once the ABI is declared stable, changes to function signatures,
JSON response schemas, or the `tinyone.h` layout are breaking changes and
require a major version bump.

### 1. Stable JSON response schema

The four response shapes (`ok/value`, `ok/false + compile`, `ok/false + runtime`,
`ok/false + panic`) are locked. What is not yet locked:

- The exact keys in the `value` object for each entry point (e.g., `memory`,
  `heap_before_shutdown`, `heap_after_shutdown` in `run_source_json`).
- The `memory` array encoding for each `Value` variant.

**Required action:** Audit every `ok: true` response body. Write a JSON schema
file (`tinyone-response-schema.json`) that captures every key and type. Add
contract tests in `TinyOne/tests/abi_api_soundness.rs` that assert the exact schema
is present for a representative success response from each entry point. Freeze
the schema at that point.

### 2. `Program` field visibility

All fields of `Program`, `Function`, `StructDef`, and `ModuleDef` are `pub`.
This creates a TOCTOU window: code outside the crate can mutate a program after
verification but before execution, bypassing the verifier's safety guarantee.

**Required action:** Scope all fields to `pub(crate)`. Add constructor or
builder methods for any external tests that currently build `Program` by hand
(grep `Program {` in test files to find them). The one known exception is the
`to_artifact` path — ensure it stays reachable through the artifact method.

### 3. `VerifiedProgram` adoption on execution paths

`VerifiedProgram` exists as a newtype wrapper produced by
`BytecodeVerifier::verify`, but the execution entry points in `runner.rs`,
`jit/cache.rs`, and the FFI layer accept raw `&Program` rather than
`&VerifiedProgram`. The type system does not enforce that every execution path
verified the program before running it.

**Required action:** Change `VM::new_unchecked`, `JitCache::*_unchecked`, and
all internal dispatch to accept `&VerifiedProgram` rather than `&Program`. The
`runner.rs` call to `BytecodeVerifier::verify` produces a `VerifiedProgram`;
thread it through to the execution backends. This makes it impossible to call
the unchecked constructors without a `VerifiedProgram` token, eliminating the
bypass risk at compile time.

### 4. `tinyone_free_string` catch_unwind guard

`tinyone_free_string` calls `CString::from_raw` without a `catch_unwind` guard.
In practice `CString::from_raw` cannot panic on a valid pointer, but the
contract that "every `extern "C"` function is panic-safe" is not uniformly
enforced.

**Required action:** Wrap the body of `tinyone_free_string` in
`catch_unwind(AssertUnwindSafe(|| { … }))`. An unwind from `from_raw` is
undefined behavior regardless; the guard ensures any future code added to the
function (logging, statistics) is also covered. This is a small mechanical
change.

### 5. Void `extern "C"` entry point policy

The current `respond()` helper cannot be used for `void`-returning FFI
functions. Any future void entry points added before v1 must install their own
`catch_unwind` guard or be converted to return `char *` with a status JSON.

**Required action:** Decide the policy before v1: either (a) require all FFI
entry points to return `char *` and use `respond()`, or (b) document a
`void_respond()` helper and use it consistently. Document the chosen policy in
`docs/ffi/c-integration.md` and `docs/contributing.md`. Enforce it in code
review.

---

## Blocking: Language Correctness

These are behavioral gaps or ambiguities that must be resolved before v1.

### 6. Integer overflow behavior

TinyLang now has `i64` literals plus first-class `u8`, `u16`, and `u32` runtime
values for low-level buffer work. Arithmetic overflow traps with
`Runtime.Memory_Overflow` in both VM and JIT paths. Remaining v1 work here is a
static type-checking surface for annotated slots and function signatures.

**Required action:** Decide the overflow model. Audit `runtime/arithmetic.rs`
and all `AddInt`, `SubInt`, `MulInt`, `DivInt` hot paths in `jit/vm.rs` to
ensure they implement the chosen model uniformly. Document the model in the
language reference.

### 7. String index bounds at compile time vs runtime

`INDEX` on a string returns the byte at `index`, with a runtime error on
out-of-bounds. There is no compile-time mechanism to express string-indexed
access safely. This is acceptable for v1 as a design constraint but must be
explicitly documented: TinyLang does not have bounds-checked string indexing at
compile time; all bounds errors are runtime errors.

**Required action:** Add a language reference section that documents this
explicitly. No code change required if the decision is "runtime only."

---

## Required Before Stable ABI: Test Coverage

### 8. Parity tests for all Phase-2 stdlib bridge builtins

`TinyOne/tests/stdlib_parity.rs` covers the stdlib modules in general but some
Phase-2 builtins have minimal or no direct test coverage. Before v1 every
builtin must have at least one positive and one negative (error) test through
`run_source`.

**Required action:** Audit `BUILTINS` (slots 35+) against `stdlib_parity.rs`.
For each uncovered builtin, add a positive test and an error-case test. Target:
zero uncovered slots.

### 9. Artifact round-trip tests for all resource limits

The artifact resource limits (14 limits documented in `docs/bytecode.md`) are
enforced in `Program::from_artifact`. Most limits have no dedicated test that
hits the exact boundary (limit - 1 should succeed; limit should fail; limit + 1
should fail).

**Required action:** Add boundary tests in `TinyOne/tests/abi_api_soundness.rs`
for each of the 14 limits. At minimum: one test that constructs an artifact at
the limit and expects success, and one that exceeds the limit and expects a
compile error.

---

## Non-Blocking: Language Limitations

These are known language gaps that will not be fixed before v1 but must be
explicitly documented as out-of-scope for v1.

| Gap | Status |
| --- | --- |
| No closures or first-class functions | By design; functions are top-level only |
| No static type checker | By design; TinyLang is dynamically typed |
| No generics or templates | Out of scope for v1 |
| No exceptions or structured error propagation | Use `result`/`option` stdlib |
| No garbage collector | Manual heap with `unsafe free`; by design |
| No tail-call optimization | Not planned for v1 |
| No REPL | Not planned for v1 |
| No debugger or stepping interface | Not planned for v1 |
| Non-ASCII identifiers not supported | Lexer limitation; by design for v1 |
| Integer overflow wraps silently | See item 6 above |

These limitations are not bugs; they are the design envelope of TinyOne v1. A
language reference section should list them explicitly so users do not
misinterpret them as issues.

---

## Non-Blocking: Go and C++ Implementations

The README notes that future maintained implementations are planned for Go and
C++. These are explicitly out of scope for v1. The Rust implementation is the
sole reference implementation until further notice.

---

## Release Checklist

The following steps must be completed in order before tagging v1:

1. All six blocking items above resolved and tested
2. `cargo test --manifest-path TinyOne/Cargo.toml` — all tests pass
3. `cargo test --manifest-path TinyOne/Cargo.toml --features testing-hooks` — all
   language fixture tests pass
4. `cargo clippy --manifest-path TinyOne/Cargo.toml --all-targets -- -D warnings`
   — zero warnings
5. `cargo fmt --manifest-path TinyOne/Cargo.toml --all --check` — no formatting
   issues
6. `tinyone.h` header reviewed and frozen; `docs/ffi/c-integration.md`
   matches exactly
7. JSON response schema file committed and contract tests pass
8. `docs/language-reference.md` written and covering at minimum: types, all
   operators, all statements, module system, builtin categories, overflow
   behavior, bounds-checking behavior
9. CHANGELOG or release notes written
10. Version bumped from `0.5.0` to `1.0.0` in `TinyOne/Cargo.toml`
11. Git tag `v1.0.0` pushed; ABI stability formally declared in
    `docs/ffi/c-integration.md` and README
