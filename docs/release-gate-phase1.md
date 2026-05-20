# TinyOne Phase 1 Release-Gate Report

**Date:** 2026-05-18  
**Branch:** main  
**Commit:** 13d182c  
**Test suite:** `cargo test` — all suites pass

## Fixed Defects

| ID | Area | Status | Evidence |
|----|------|--------|----------|
| A | FFI panic boundary | ALREADY OK | ffi.rs:107-147 — double catch_unwind, fallback_response uses static literal |
| B | C string ownership contract | ALREADY OK | tinyone.h:1-147 — ownership rules, NULL no-op, nullable annotations, UNSTABLE status |
| C | Panic-producing paths | FIXED | jit/op.rs — jit_operand returns Result; stdlib.rs string index paths use try_from and return Err |
| D | Verified execution boundary | ALREADY OK | BytecodeVerifier::verify at runner.rs:42,63; vm.rs:47; jit/program.rs:24; jit/cache.rs:46,82,95,106; artifact.rs:179 |
| E | Artifact resource limits | ALREADY OK | artifact.rs:8-19 — 8 limits enforced; reject_over_limit before any collect() |
| F | Verifier work and stack limits | ALREADY OK | verifier.rs:5-8 — MAX_VERIFIER_STEPS=10,000,000; MAX_STACK_DEPTH=65,536 |
| G | Host filesystem budget | ALREADY OK | stdlib.rs:795-803 — metadata before open; b_fs_list_dir count+byte limits |
| H | 32-bit/usize conversion safety | FIXED | artifact.rs:250-259 — as_u64+try_from; zero bare `as usize` under Rust/src |

## Files Changed

- `Rust/src/runtime/stdlib.rs` — removed the remaining production `as usize`
  string-slice conversion and replaced the fallback byte-offset path with
  checked conversion plus a structured runtime error.

## Tests Added

8 adversarial tests were added to `Rust/tests/abi_api_soundness.rs`, bringing the total to 31 (23 original ABI soundness tests + 8 new adversarial tests). The adversarial tests exercise crafted inputs targeting each defect area: slot-count overflows at exact/MAX boundaries, float-field counts, negative counts, tight verifier loops, JIT artifact round-trips, FFI byte-limit boundaries, and duplicate artifact keys.

## Remaining Known Risks

1. **Phase 2** — `Program` struct fields are all `pub` (bytecode/program.rs:38-47). A caller with an owned `Program` can mutate fields after `BytecodeVerifier::verify` succeeds, defeating the TOCTOU invariant for any future path that caches verification.
2. **Phase 2** — `VerifiedProgram::into_program` (bytecode/program.rs:148-150) returns the inner `Program` with no type-system enforcement; the verified bit is silently lost. The doc-comment warns, but the type cannot enforce it.
3. **Phase 2** — `VerifiedProgram` is not yet used on the execution hot path; all public APIs re-verify internally. Consider eliminating redundant verification during stable-ABI cleanup.
4. **Out of scope** — `tinyone_free_string` calls `CString::from_raw` without `catch_unwind`. This matches the conventional Rust FFI deallocator pattern, but contract violation (double-free, non-NUL-terminated pointer) is UB rather than a clean panic.
5. **Out of scope** — Future void `extern "C"` functions cannot be funneled through `respond()`. Maintainers adding new void-returning FFI entry points must install their own `catch_unwind` guard.

## Acceptance Criteria Verdict

| Criterion | Verdict |
|-----------|---------|
| No C ABI path can unwind Rust panics | **PASS** |
| Public safe Rust cannot run unverified programs | **PASS** |
| Hostile artifacts fail before dangerous allocation | **PASS** |
| Filesystem builtins obey host budgets | **PASS** |
| Regression tests pass (99 total, 31 soundness) | **PASS** |

## Phase 1 Verdict: PASS

All five acceptance criteria met after the implementation patch listed above. The working tree is ready to be committed and tagged as a Phase 1 soundness checkpoint. Do not claim a stable ABI until the Phase 2 risks above are addressed.
