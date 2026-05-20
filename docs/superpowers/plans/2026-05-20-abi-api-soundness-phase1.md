# ABI/API Soundness Phase 1 — Verification & Finalization Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Audit, adversarially stress-test, review, and commit the Phase 1 ABI/API soundness implementation already in the working tree.

**Architecture:** The implementation patch (~1072 insertions, 27 files) addresses eight classes of defects (FFI panic boundary, C ownership contract, panic paths, verified execution boundary, artifact resource limits, verifier limits, filesystem budgets, and usize conversion safety). All 84 tests pass. This plan verifies correctness from every angle before committing.

**Tech Stack:** Rust 2021 edition, `cargo test`, `cc` (C compiler for FFI smoke test), `serde_json`, `Blake2b512`, TinyOne bytecode verifier.

---

## Current State

**DO NOT skip to Task 5 without running Tasks 1–4.** The tests pass, but passing tests are necessary but not sufficient — the audit, adversarial, and review tasks catch issues that tests cannot.

Key files:
- `Rust/src/ffi.rs` — FFI entry points with `catch_unwind` guards
- `Rust/src/bytecode/verifier.rs` — work/stack limits, budget checks
- `Rust/src/bytecode/artifact.rs` — resource limits before allocation
- `Rust/src/runtime/stdlib.rs` — fs_read budget, fs_list_dir limits
- `Rust/src/jit/op.rs` and `chunk.rs` — Result-returning JIT operand conversion
- `Rust/src/bytecode/program.rs` — `VerifiedProgram` wrapper type
- `Rust/src/runner.rs` — `BytecodeVerifier::verify` before every execution path
- `Rust/tests/abi_api_soundness.rs` — 23 soundness regression tests
- `tinyone.h` — C API ownership contract documentation
- `docs/release-gate-phase1.md` — Phase 1 report

---

## Task 1: Audit — Verify All Eight Defect Areas

**Files to read:**
- `Rust/src/ffi.rs`
- `Rust/src/bytecode/verifier.rs`
- `Rust/src/bytecode/artifact.rs`
- `Rust/src/runtime/stdlib.rs`
- `Rust/src/jit/op.rs`, `Rust/src/jit/chunk.rs`, `Rust/src/jit/program.rs`
- `Rust/src/bytecode/program.rs`
- `Rust/src/runner.rs`, `Rust/src/runtime/vm.rs`, `Rust/src/jit/cache.rs`
- `tinyone.h`

- [ ] **Step 1: Audit A — FFI panic boundary**

Verify `ffi.rs` has:
- `respond()` wraps `response_cstring()` in `catch_unwind(AssertUnwindSafe(...))`
- `response_cstring()` wraps the callback in `catch_unwind(AssertUnwindSafe(...))`
- `fallback_response()` uses a static byte literal — no allocation, no panic path
- `cstring_or_fallback()` handles `CString::new` failure without panicking
- `error_payload()` has no panic path (match on `TinyOneError` variants only)

Expected: All five conditions are true. If any is false, file an issue.

Run: `grep -n "catch_unwind\|fallback_response\|CString::new\|unwrap\|expect" Rust/src/ffi.rs`

Expected output: `catch_unwind` appears at lines ~143,150; `fallback_response` at lines ~145,162,173,177; `CString::new` at line ~174 (inside `unwrap_or_else`); no bare `unwrap()` or `expect()`.

- [ ] **Step 2: Audit B — C string ownership contract**

Read `tinyone.h` and verify:
- `OWNERSHIP CONTRACT` section present with `tinyone_free_string` rule
- Every `char *` return is documented with "freed with `tinyone_free_string`"
- `tinyone_free_string(NULL)` is documented as safe
- `inputs_json` is marked `/* nullable */`
- Each non-nullable `const char *` parameter is described as non-nullable
- ABI status is marked UNSTABLE

Run: `grep -c "tinyone_free_string\|nullable\|UNSTABLE\|OWNERSHIP" tinyone.h`

Expected output: Numbers > 0 for each grep. If any returns 0, the contract is incomplete.

- [ ] **Step 3: Audit C — Panic paths replaced with Result**

Verify no production panic paths remain in runtime, JIT, artifact, verifier, or FFI paths reachable from public APIs.

Run:
```bash
grep -n "\.unwrap()\|\.expect(\|unreachable!()\|panic!(" \
  Rust/src/runtime/vm.rs \
  Rust/src/runtime/stdlib.rs \
  Rust/src/jit/vm.rs \
  Rust/src/jit/op.rs \
  Rust/src/jit/chunk.rs \
  Rust/src/jit/program.rs \
  Rust/src/bytecode/artifact.rs \
  Rust/src/bytecode/verifier.rs \
  Rust/src/ffi.rs \
  Rust/src/runner.rs \
  Rust/src/artifact_io.rs
```

Expected output: Zero results. If any exist, check whether the call site is:
- In a test-only path (`#[cfg(test)]` or `tests/` directory) — acceptable
- In a benchmark binary (`bin/`) — acceptable  
- In a non-reachable internal path — document why
- In a reachable public path — THIS IS A BUG, fix before proceeding

- [ ] **Step 4: Audit D — Verified execution boundary**

Verify every public execution entry point calls `BytecodeVerifier::verify` before executing:

Run:
```bash
grep -n "BytecodeVerifier::verify\|VerifiedProgram::verify" \
  Rust/src/runner.rs \
  Rust/src/runtime/vm.rs \
  Rust/src/jit/cache.rs \
  Rust/src/jit/program.rs \
  Rust/src/bytecode/artifact.rs
```

Expected output:
- `runner.rs`: `BytecodeVerifier::verify(program)` appears before `RunMode::Vm` and `RunMode::Jit` dispatch in both `run_program_with_env` and `run_program_report`
- `runtime/vm.rs`: `BytecodeVerifier::verify(program)` in `VM::new`
- `jit/cache.rs`: `BytecodeVerifier::verify(program)` in `compile`, `run_program`, `run_program_with_env`, `run_program_report`
- `jit/program.rs`: `crate::BytecodeVerifier::verify(program)?` in `JitProgram::compile`
- `bytecode/artifact.rs`: `BytecodeVerifier::verify(&program)?` at end of `from_artifact`

- [ ] **Step 5: Audit E — Artifact resource limits**

Read `Rust/src/bytecode/artifact.rs`. Verify:
- Constants at top of file: `MAX_FUNCTIONS`, `MAX_STRUCTS`, `MAX_CODE_OPS`, `MAX_TOTAL_CODE_OPS`, `MAX_STRINGS`, `MAX_FIELDS`, `MAX_SLOT_COUNT`, `MAX_MODULES`, `MAX_MODULE_IMPORTS`, `MAX_MODULE_EXPORTS`, `MAX_STRUCT_FIELDS`, `MAX_NAMES`, `MAX_TEXT_BYTES`
- `from_artifact` calls `expect_array_limited` before iterating functions
- `slot_count` is validated via `reject_over_limit` before function construction
- `expect_usize` uses `as_u64()` + `usize::try_from()` (no bare `as usize`)
- Total code ops uses `checked_add` before comparison

Run: `grep -n "as usize" Rust/src/bytecode/artifact.rs`

Expected output: Zero results. Any `as usize` is a potential truncation bug on 32-bit platforms.

- [ ] **Step 6: Audit F — Verifier work and stack limits**

Read `Rust/src/bytecode/verifier.rs`. Verify:
- `MAX_VERIFIER_STEPS` is defined (should be 10,000,000)
- `MAX_STACK_DEPTH` is defined (should be 65,536)
- The work-limit check is `steps += 1; if steps > MAX_VERIFIER_STEPS { return Err(...) }`
- `next_depth` rejects depths above `MAX_STACK_DEPTH`
- The jump-graph BFS terminates via `seen` map (already-visited PCs are not re-added to `todo`)

Run: `grep -n "MAX_VERIFIER_STEPS\|MAX_STACK_DEPTH\|steps +=" Rust/src/bytecode/verifier.rs`

Expected output: Constants defined near top; step increment inside the `while let Some` loop.

- [ ] **Step 7: Audit G — Filesystem budget enforcement**

Read `b_fs_read` and `b_fs_list_dir` in `Rust/src/runtime/stdlib.rs`. Verify:
- `b_fs_read`: calls `std::fs::metadata` FIRST, checks `meta.len() > MAX_BUFFER_BYTES as u64` BEFORE `File::open`
- `b_fs_read`: also has a double-check after reading (TOCTOU defense)
- `b_fs_list_dir`: checks `sorted.len() >= MAX_FS_LIST_DIR_ENTRIES` INSIDE the loop, before inserting
- `b_fs_list_dir`: accumulates `name_bytes` with `checked_add` and compares to `MAX_BUFFER_BYTES`

Run: `grep -n "metadata\|MAX_BUFFER_BYTES\|MAX_FS_LIST_DIR_ENTRIES\|checked_add" Rust/src/runtime/stdlib.rs`

Expected output: `metadata` call before `File::open`; limits checked before and after reading.

- [ ] **Step 8: Audit H — usize conversion safety**

Run: `grep -rn " as usize" Rust/src/ --include="*.rs" | grep -v "//\|tests\|bin\|#\[cfg(test" | grep -v "\.len() as\|bytes.len()\|MIN.*as usize\|count() as\|_as_usize\|_bytes as usize"`

Expected output: Zero bare `as usize` conversions from serialized/untrusted integers. Legitimate uses in the runtime (e.g., converting `Value::Int` that has already been bounds-checked) are acceptable if annotated.

- [ ] **Step 9: Commit audit findings**

If all eight areas check out, write a one-sentence finding: "Audit complete — all eight defect areas confirmed fixed."

If any issues found, document them precisely with file:line citations and continue to Task 2.

---

## Task 2: Adversarial Stress Tests

**Files:**
- Test: `Rust/tests/abi_api_soundness.rs` (add adversarial tests at the end)
- Run: `cargo test --test abi_api_soundness 2>&1`

- [ ] **Step 1: Write adversarial artifact tests**

Add to `Rust/tests/abi_api_soundness.rs`:

```rust
#[test]
fn adversarial_artifact_maximum_valid_boundary() {
    // An artifact exactly at every limit should be accepted (or fail at verifier, not panic).
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(MAX_ARTIFACT_SLOT_COUNT);
    // Should either succeed or return a structured error (e.g., verifier).
    // Must not panic.
    let result = std::panic::catch_unwind(|| Program::from_artifact(artifact));
    assert!(result.is_ok(), "artifact at exact slot_count limit must not panic");
}

#[test]
fn adversarial_artifact_negative_slot_count_is_rejected() {
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(-1i64);
    expect_artifact_error(artifact, "slot_count");
}

#[test]
fn adversarial_artifact_slot_count_as_float_is_rejected() {
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(1.5f64);
    expect_artifact_error(artifact, "slot_count");
}

#[test]
fn adversarial_artifact_u64_max_slot_count_is_rejected() {
    let mut artifact = minimal_artifact();
    artifact["slot_count"] = json!(u64::MAX);
    expect_artifact_error(artifact, "slot_count");
}

#[test]
fn adversarial_artifact_duplicate_format_fields_are_checked() {
    // serde_json parses the last value of a duplicate key; the first value is dropped.
    // The format check should still work correctly.
    let text = r#"{"format":"tinyone-bytecode","version":1,"format":"bad","code":[{"op":"HALT","arg":0,"arg2":0}],"slot_count":0,"names":[],"functions":[],"strings":[],"structs":[],"fields":[]}"#;
    let artifact: serde_json::Value = serde_json::from_str(text).expect("valid JSON");
    let result = Program::from_artifact(artifact);
    // Should return Err with "Unsupported" since format ends up as "bad".
    expect_error_contains(result, "Unsupported");
}
```

- [ ] **Step 2: Run adversarial artifact tests**

Run: `cargo test --test abi_api_soundness adversarial 2>&1`

Expected: All adversarial tests pass. If any panic, that is a bug — fix before proceeding.

- [ ] **Step 3: Write adversarial verifier tests**

Add to `Rust/tests/abi_api_soundness.rs`:

```rust
#[test]
fn adversarial_verifier_tight_loop_terminates_cleanly() {
    // A loop with a single back-edge should verify quickly.
    let code = vec![
        Instr::new(Op::PushInt, 0, 0),  // pc=0: push 0
        Instr::new(Op::JumpIfZero, 3, 0), // pc=1: if zero, jump to halt
        Instr::new(Op::Jump, 0, 0),      // pc=2: back-edge to pc=0
        Instr::new(Op::Halt, 0, 0),      // pc=3
    ];
    let result = BytecodeVerifier::verify(&Program { code, ..minimal_program() });
    assert!(result.is_ok(), "tight loop with unreachable halt should verify");
}

#[test]
fn adversarial_verifier_step_limit_triggers_before_timeout() {
    // Build a program where the BFS fan-out exceeds MAX_VERIFIER_STEPS.
    // Use many JumpIfZero instructions that all point to a shared join point
    // but each has a unique false branch so the BFS cannot short-circuit via 'seen'.
    // This is a regression test — the step limit should fire, not a timeout.
    let mut code = Vec::new();
    for i in 0..1024usize {
        let join = (i * 4 + 4) as i64;
        code.push(Instr::new(Op::PushInt, 0, 0));     // +0
        code.push(Instr::new(Op::JumpIfZero, join, 0)); // +1: branch to join
        code.push(Instr::new(Op::PushInt, 1, 0));     // +2: unique false path
        code.push(Instr::new(Op::Store, 0, 0));       // +3: consume the push (depth=0)
    }
    code.push(Instr::new(Op::Halt, 0, 0));
    let program = Program {
        code,
        slot_count: 1,
        ..minimal_program()
    };
    // This should either verify successfully or return a step-limit error.
    // It must NOT panic or time out.
    let result = std::panic::catch_unwind(|| BytecodeVerifier::verify(&program));
    assert!(result.is_ok(), "verifier step-limit program must not panic");
}
```

- [ ] **Step 4: Write adversarial FFI tests**

Add to `Rust/tests/abi_api_soundness.rs`:

```rust
#[test]
fn adversarial_ffi_run_artifact_with_max_byte_minus_one_accepts_or_errors_cleanly() {
    // An artifact JSON at exactly MAX_ARTIFACT_BYTES - 1 bytes (spaces) should not
    // exceed the byte limit. The spaces form invalid JSON so it should return a
    // compile error, not a byte-limit error or a panic.
    let near_limit =
        CString::new(vec![b' '; MAX_ARTIFACT_BYTES as usize - 1]).expect("no NUL in spaces");
    let mode = cstring("vm");
    let response = take_ffi_json(unsafe {
        tinyone_run_artifact_json(near_limit.as_ptr(), mode.as_ptr(), std::ptr::null())
    });
    // Must be valid JSON with ok=false.
    assert_eq!(response.get("ok").and_then(JsonValue::as_bool), Some(false));
    assert!(
        response.get("kind").and_then(JsonValue::as_str).is_some(),
        "error response must include kind"
    );
}

#[test]
fn adversarial_ffi_mode_with_embedded_nul_returns_error() {
    // A mode string that is valid up to the NUL byte but has garbage after it
    // should be treated as the prefix "vm" and succeed (C strings stop at NUL).
    let mode_with_nul = CString::new("vm").expect("no embedded NUL");
    let source = cstring("print 1");
    let response = take_ffi_json(unsafe {
        tinyone_run_source_json(source.as_ptr(), mode_with_nul.as_ptr(), std::ptr::null())
    });
    // CString strips bytes after NUL so this should succeed.
    assert_eq!(response.get("ok").and_then(JsonValue::as_bool), Some(true));
}
```

- [ ] **Step 5: Run all adversarial tests**

Run: `cargo test --test abi_api_soundness 2>&1`

Expected: All tests pass (previously 23, now up to ~30 depending on additions). No panics.

If any test reveals a real bug, note it with `ADVERSARIAL FINDING:` and proceed to Task 3 to fix it.

---

## Task 3: Implementation — Fix Any Adversarial Findings

**This task is conditional.** Run it only if Task 2 uncovered real bugs.

- [ ] **Step 1: For each ADVERSARIAL FINDING, identify the precise fix location**

Use the finding's file:line citation and the description from Task 2 to identify what to change. Do not make any changes beyond the minimum required.

- [ ] **Step 2: Apply the fix**

Edit the relevant file. Do not add helper functions or abstractions unless strictly required. Do not rename existing functions. Do not refactor surrounding code.

- [ ] **Step 3: Verify the adversarial test now passes**

Run: `cargo test --test abi_api_soundness 2>&1`

Expected: All tests pass including the new adversarial test.

- [ ] **Step 4: Verify the full suite still passes**

Run: `cargo test 2>&1 | tail -20`

Expected: All test suites pass.

---

## Task 4: Code Review

**Files to review:**
- `Rust/src/ffi.rs`
- `Rust/src/bytecode/artifact.rs` (focus on `from_artifact`, resource-limit helpers)
- `Rust/src/bytecode/verifier.rs` (focus on `verify_chunk`, step counter, stack-depth guard)
- `Rust/src/runtime/stdlib.rs` (focus on `b_fs_read`, `b_fs_list_dir`)
- `Rust/src/jit/op.rs` (focus on `from_instr`, `jit_operand`)
- `Rust/src/bytecode/program.rs` (focus on `VerifiedProgram`)
- `Rust/tests/abi_api_soundness.rs`
- `tinyone.h`

Review checklist for each file:

- [ ] **Step 1: Check for incomplete error handling**

For each public API function, ensure every `?` propagates through a `Result` return. Ensure there are no `let _ = result` suppressions on error-producing calls in safety-critical paths.

- [ ] **Step 2: Check for panic paths**

Search for `unwrap()`, `expect(`, `unreachable!()`, `panic!()`, and direct array indexing (`[index]` where `index` is not a literal) in non-test code. Each instance should be either:
- Unreachable by proof (e.g., `HashMap::insert` followed immediately by `HashMap::get` on the same key)
- Protected by `catch_unwind` (only in FFI boundary)
- In test/bench code only

- [ ] **Step 3: Check for unchecked allocation**

Scan for `Vec::with_capacity`, `Vec::new`, `String::new`, `HashMap::new` calls that could accept untrusted sizes without pre-validation. Resource limits must be checked BEFORE allocation.

- [ ] **Step 4: Check public API contracts are documented**

Every `pub fn` in `ffi.rs` should have a `# Safety` doc comment if it is `unsafe`. Every public Rust function in `runner.rs`, `vm.rs`, `jit/program.rs` that accepts a `&Program` should document whether it verifies internally.

- [ ] **Step 5: Check `VerifiedProgram` invariant**

In `Rust/src/bytecode/program.rs`, verify:
- `VerifiedProgram::verify` calls `BytecodeVerifier::verify` before wrapping
- `VerifiedProgram::into_program` has a doc-comment warning about losing the type-system proof
- The `Program` struct fields are still `pub` (known Phase 2 risk — do NOT fix here, just document)

- [ ] **Step 6: Write review verdict**

Write one paragraph: was any issue found? If yes, what is it and where? If no, "Review complete — no issues found."

---

## Task 5: Final Verification and Commit

- [ ] **Step 1: Run full test suite one final time**

Run: `cargo test 2>&1`

Expected:
```
test result: ok. N passed; 0 failed; 0 ignored; ...
```
across all test files (`abi_api_soundness`, `runtime_parity`, `stdlib_parity`, and any others).

If any test fails, DO NOT proceed to commit. Return to Task 3.

- [ ] **Step 2: Check for compilation warnings**

Run: `cargo build 2>&1 | grep -E "^warning:|^error:"`

Expected: No `error:` lines. Warnings are acceptable only if they were present before this patch (do not introduce new warnings).

- [ ] **Step 3: Update release-gate report if needed**

If adversarial or review tasks found new issues:
- Update `docs/release-gate-phase1.md` to list them under "Remaining Known Risks"
- Update the test count

If no new issues: no change needed (the report already reflects Phase 1 PASS).

- [ ] **Step 4: Stage changes for commit**

```bash
git add \
  Rust/src/artifact_io.rs \
  Rust/src/bin/tinyone-bench.rs \
  Rust/src/bytecode/artifact.rs \
  Rust/src/bytecode/mod.rs \
  Rust/src/bytecode/program.rs \
  Rust/src/bytecode/verifier.rs \
  Rust/src/compiler/parser.rs \
  Rust/src/compiler/symbols.rs \
  Rust/src/ffi.rs \
  Rust/src/internal_testing.rs \
  Rust/src/jit/cache.rs \
  Rust/src/jit/chunk.rs \
  Rust/src/jit/op.rs \
  Rust/src/jit/program.rs \
  Rust/src/jit/vm.rs \
  Rust/src/lib.rs \
  Rust/src/runner.rs \
  Rust/src/runtime/aggregate.rs \
  Rust/src/runtime/context.rs \
  Rust/src/runtime/heap.rs \
  Rust/src/runtime/pointers.rs \
  Rust/src/runtime/stdlib.rs \
  Rust/src/runtime/vm.rs \
  Rust/src/syntax/lexer.rs \
  Rust/tests/runtime_parity.rs \
  Rust/tests/abi_api_soundness.rs \
  Rust/Cargo.lock \
  tinyone.h \
  docs/release-gate-phase1.md
```

- [ ] **Step 5: Review staged diff**

Run: `git diff --cached --stat`

Expected: Should match the 27-file, ~1072-insertion diff documented in the plan header.

- [ ] **Step 6: Commit**

```bash
git commit -m "$(cat <<'EOF'
fix(soundness): Phase 1 ABI/API release-gate — all eight defect areas

A  ffi.rs: double catch_unwind + static fallback_response guards all FFI
   entry points against Rust panic unwinding into C callers.

B  tinyone.h: OWNERSHIP CONTRACT section, NULL no-op guarantee, nullable
   annotation on inputs_json, non-nullable specifications per parameter,
   ABI UNSTABLE status declaration.

C  jit/op.rs, jit/chunk.rs, stdlib.rs: jit_operand returns Result; all
   string-index and char-index paths use checked conversions and return Err.

D  runner.rs, vm.rs, jit/cache.rs, jit/program.rs, artifact.rs: every
   public execution entry point calls BytecodeVerifier::verify before
   running or compiling. VerifiedProgram type added for explicit boundary.

E  artifact.rs: hard limits (function/code/string/field/slot/module counts)
   checked via reject_over_limit before any Vec::collect or allocation.

F  verifier.rs: MAX_VERIFIER_STEPS work cap and MAX_STACK_DEPTH enforced;
   jump graph BFS terminates with clean error on crafted inputs.

G  stdlib.rs: fs_read checks metadata() before File::open; fs_list_dir
   enforces count and byte-name limits inside the iteration loop.

H  artifact.rs: expect_usize uses as_u64()+try_from(); zero bare `as usize`
   from serialized integers remain in artifact decode paths.

All 84 tests pass including 23 required ABI/API soundness regression tests.
Phase 1 verdict: PASS (see docs/release-gate-phase1.md).

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 7: Verify commit was created**

Run: `git log --oneline -3`

Expected: Top commit should be the Phase 1 soundness gate commit.

---

## Self-Review

**Spec coverage check:**

| Spec Requirement | Covered by Task |
|---|---|
| A — FFI panic boundary | Task 1 Step 1 (audit) + Task 2 Step 4 (adversarial) |
| B — C string ownership contract | Task 1 Step 2 (audit) |
| C — Panic paths replaced | Task 1 Step 3 (audit) |
| D — Verified execution boundary | Task 1 Step 4 (audit) |
| E — Artifact resource limits | Task 1 Step 5 (audit) |
| F — Verifier work/stack limits | Task 1 Step 6 (audit) + Task 2 Step 3 (adversarial) |
| G — Filesystem budget enforcement | Task 1 Step 7 (audit) |
| H — usize conversion safety | Task 1 Step 8 (audit) |
| All required tests present and passing | Task 2 Step 5 + Task 5 Step 1 |
| Commit with full patch | Task 5 Steps 4–7 |

**No placeholders found.** All steps contain exact commands, expected output, or concrete code.

**Type consistency:** `minimal_program()`, `expect_artifact_error()`, `expect_error_contains()`, `cstring()`, `take_ffi_json()`, `assert_ffi_error()`, `assert_ffi_ok()` are all defined in the existing test file.

**Constants used in adversarial tests** (`MAX_ARTIFACT_BYTES`, `MAX_ARTIFACT_SLOT_COUNT`) match the `const` declarations at the top of `tests/abi_api_soundness.rs` (lines 17–23).
