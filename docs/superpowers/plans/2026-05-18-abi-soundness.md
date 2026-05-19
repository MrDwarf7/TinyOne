# TinyOne ABI/API Soundness Gate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Verify, harden, and gate-check all eight release-blocking soundness defects (A–H) in the TinyOne Rust library before any stable ABI claim is made, then produce a signed release-gate report.

**Architecture:** Five sequenced agents — Audit and Adversarial run in parallel (read-only), then Implementation fills any identified gaps, Test adds regression coverage for those gaps, and Review produces the final pass/fail verdict. All source work happens under `Rust/`. The working tree already contains implementations targeting all eight defect areas and 15 passing tests in `Rust/tests/abi_api_soundness.rs`; agents verify completeness, probe for remaining issues, and close any gaps.

**Tech Stack:** Rust 2021, `serde_json`, `blake2`, `hex`; `cargo test` for all test execution; `tinyone` library crate rooted at `Rust/Cargo.toml`.

---

## File Map

| File | Role |
|------|------|
| `Rust/src/ffi.rs` | All `extern "C"` entry points; panic boundary (A) |
| `tinyone.h` | C header; ownership/nullability documentation (B) |
| `Rust/src/runtime/vm.rs` | `VM::new` — must call verifier before accepting program (D) |
| `Rust/src/runtime/stdlib.rs` | `b_fs_read`, `b_fs_list_dir` — filesystem budget enforcement (G) |
| `Rust/src/bytecode/verifier.rs` | Work cap, stack depth limit (F) |
| `Rust/src/bytecode/artifact.rs` | Resource limits before allocation (E, H) |
| `Rust/src/bytecode/program.rs` | `VerifiedProgram` newtype; `Program` public fields (D) |
| `Rust/src/runner.rs` | `run_program`, `run_program_report`, `run_program_with_env` — must call verifier (D) |
| `Rust/src/jit/program.rs` | `JitProgram::compile` — must call verifier, return `Result` (C, D) |
| `Rust/src/jit/cache.rs` | `JitCache::compile`, `run_program` — must call verifier (D) |
| `Rust/src/jit/op.rs` | `JitOp::from_instr` — converts i64 operands via `jit_operand` (C, H) |
| `Rust/src/jit/chunk.rs` | `JitChunk::compile` — must not panic on bad operands (C) |
| `Rust/src/artifact_io.rs` | `load_artifact` — byte-size check before read (E) |
| `Rust/src/lib.rs` | Public API surface — exports to audit (D) |
| `Rust/tests/abi_api_soundness.rs` | All 15 required regression tests |

---

## Task 1 — Audit Agent  *(parallel with Task 2)*

**What this task does:** Reads each A–H source location and verifies the fix is complete, correctly wired, and has no reachable panic path from a public API. Produces a structured findings report.

**Files to read:** all files in the File Map above.

**Findings format:** For each defect, record one of:
- `OK file:line — <evidence>`
- `GAP file:line — <what is missing>`
- `PARTIAL file:line — <what is done / what remains>`

---

### A — FFI Panic Boundary (`Rust/src/ffi.rs`)

- [ ] **Read `ffi.rs` lines 107–147** and verify the following invariants hold:

  1. `respond()` wraps `response_cstring()` in `catch_unwind(AssertUnwindSafe(...))`.
     Expected pattern at ~line 108:
     ```rust
     match catch_unwind(AssertUnwindSafe(|| response_cstring(callback))) {
         Ok(response) => response.into_raw(),
         Err(_) => fallback_response().into_raw(),
     }
     ```
  2. `response_cstring()` wraps the callback in a second `catch_unwind`.
  3. `fallback_response()` uses only operations that cannot panic: a static `&[u8]` literal
     followed by `from_vec_with_nul_unchecked` (sound because the literal is known-valid).
  4. `cstring_or_fallback()` uses `unwrap_or_else`, not `.unwrap()`.
  5. No `extern "C"` function calls anything outside `respond()`.

- [ ] **Search for raw `.unwrap()` or `.expect(` in `ffi.rs`:**
  ```bash
  grep -n 'unwrap()\|\.expect(' Rust/src/ffi.rs
  ```
  Expected: no matches (only `unwrap_or_else` is acceptable).

- [ ] **Record finding A** in the findings report.

---

### B — C String Ownership Contract (`tinyone.h`)

- [ ] **Read `tinyone.h` in full** and verify:
  1. File-level comment states callers must use `tinyone_free_string`, not `free()`.
  2. `tinyone_free_string(NULL)` is documented as a no-op.
  3. Every `const char *` parameter is documented as either non-nullable or explicitly
     marked `/* nullable */`.
  4. `inputs_json` is marked `/* nullable */` in every function that accepts it.
  5. The "ABI STATUS: UNSTABLE" line is present.

- [ ] **Record finding B** in the findings report.

---

### C — Panic-Producing Paths (runtime, JIT, stdlib)

- [ ] **Check `Rust/src/jit/op.rs` for the `jit_operand` helper:**
  ```bash
  grep -n 'jit_operand\|as usize\|unwrap()' Rust/src/jit/op.rs
  ```
  Verify `jit_operand` converts `i64 → usize` via `checked_non_negative_usize` or
  `usize::try_from`, and returns `Result` on negative or overflow.

- [ ] **Check `Rust/src/jit/chunk.rs` for any raw `.unwrap()` or direct index:**
  ```bash
  grep -n 'unwrap()\|\.expect(\|as usize\|\[.*\]' Rust/src/jit/chunk.rs | grep -v '//\|test\|0usize'
  ```
  Expected: no panicking indexing without a prior bounds check.

- [ ] **Check `Rust/src/runtime/stdlib.rs` `b_str_char_at` (the `str_char_at` builtin):**
  Read the function. Verify it uses `usize::try_from` on the index (not `as usize`)
  and returns `Err` for negative indices and out-of-bounds.

- [ ] **Scan all public-path source files for bare `.unwrap()` calls that are
  reachable from a public API (not in tests or bench):**
  ```bash
  grep -rn '\.unwrap()' Rust/src/ | grep -v 'tests/\|bench\|_or\|_or_else'
  ```
  Any match that is not in test/bench code and is not `unwrap_or`/`unwrap_or_else`
  is a potential GAP. Record each match.

- [ ] **Record finding C** in the findings report.

---

### D — Verified Execution Boundary

- [ ] **Check `Rust/src/bytecode/program.rs`** for `VerifiedProgram`:
  - `VerifiedProgram::verify` calls `BytecodeVerifier::verify(&program)?` before wrapping.
  - `into_program` is the only way to extract the inner `Program`.

- [ ] **Check `Rust/src/runner.rs`** — every public `run_*` function:
  ```bash
  grep -n 'BytecodeVerifier::verify\|pub fn run' Rust/src/runner.rs
  ```
  Verify `BytecodeVerifier::verify(program)?` appears before any `VM::new` or
  `JitCache::run_program` call in `run_program_with_env` and `run_program_report`.

- [ ] **Check `Rust/src/runtime/vm.rs` `VM::new`:**
  ```bash
  grep -n 'BytecodeVerifier::verify\|pub fn new' Rust/src/runtime/vm.rs
  ```
  Verify it is the first statement of `VM::new`.

- [ ] **Check `Rust/src/jit/program.rs` `JitProgram::compile`:**
  Verify `crate::BytecodeVerifier::verify(program)?` is first.

- [ ] **Check `Rust/src/jit/cache.rs`** — `compile`, `run_program`, `run_program_report`,
  `run_program_with_env` all call `BytecodeVerifier::verify(program)?`.

- [ ] **Check `Rust/src/artifact_io.rs` `load_artifact`:**
  Verify it calls `Program::from_artifact(data)`, which itself calls
  `BytecodeVerifier::verify(&program)?` at the end.

- [ ] **Check `Rust/src/bytecode/artifact.rs` end of `from_artifact`:**
  ```bash
  grep -n 'BytecodeVerifier::verify' Rust/src/bytecode/artifact.rs
  ```
  Expected: one match at the end of `from_artifact`.

- [ ] **Record finding D** in the findings report.

---

### E — Artifact Resource Limits (`Rust/src/bytecode/artifact.rs`)

- [ ] **Read the constant block (lines 8–24)** and verify these values:
  - `MAX_FUNCTIONS = 4_096`
  - `MAX_CODE_OPS = 65_536`
  - `MAX_TOTAL_CODE_OPS = 262_144`
  - `MAX_STRINGS = 65_536`
  - `MAX_SLOT_COUNT = 65_536`
  - `MAX_MODULES = 256`
  - `MAX_MODULE_IMPORTS = 4_096`
  - `MAX_MODULE_EXPORTS = 4_096`
  - `MAX_ARTIFACT_BYTES = 8 * 1024 * 1024`

- [ ] **Verify `from_artifact` calls `reject_over_limit` BEFORE allocating** for each
  of: `functions`, `slot_count`, `strings`, `fields`, `structs`, total code ops.
  The pattern is: read count → `reject_over_limit(...)` → allocate.

- [ ] **Verify `reject_over_limit` returns `Err`, does not panic.**

- [ ] **Record finding E** in the findings report.

---

### F — Verifier Work and Stack Limits (`Rust/src/bytecode/verifier.rs`)

- [ ] **Read the constant block (lines 5–8)** and verify:
  - `MAX_VERIFIER_STEPS = 10_000_000`
  - `MAX_STACK_DEPTH = 65_536`
  - `MAX_VERIFIER_FUNCTIONS = 4_096`
  - `MAX_VERIFIER_TOTAL_OPS = 262_144`

- [ ] **Verify the BFS loop in `verify_chunk` checks `steps > MAX_VERIFIER_STEPS` on
  every iteration** and returns `Err` (not panic).

- [ ] **Verify `next_depth` returns `Err` when `depth > MAX_STACK_DEPTH`.**

- [ ] **Verify `verify_program_budget` checks total ops and function count before
  starting the per-chunk BFS.**

- [ ] **Record finding F** in the findings report.

---

### G — Host Filesystem Budget (`Rust/src/runtime/stdlib.rs`)

- [ ] **Read `b_fs_read` (~line 793)** and verify:
  1. `std::fs::metadata` is called first.
  2. `meta.len() > crate::MAX_BUFFER_BYTES as u64` → `Err` before any file open.
  3. After open, `.take((MAX_BUFFER_BYTES + 1) as u64)` caps the read.
  4. Post-read `bytes.len() > MAX_BUFFER_BYTES` check as a second guard.

- [ ] **Read `b_fs_list_dir` (~line 844)** and verify:
  1. `MAX_FS_LIST_DIR_ENTRIES = 65_536` constant is present.
  2. Entry count is checked against the limit before inserting.
  3. `name_bytes` is accumulated with `checked_add` and checked against `MAX_BUFFER_BYTES`.
  4. No `.unwrap()` or `.expect()` — all IO errors are mapped to `TinyOneError::runtime`.

- [ ] **Record finding G** in the findings report.

---

### H — 32-bit / usize Conversion Safety

- [ ] **Read `expect_usize` in `Rust/src/bytecode/artifact.rs`:**
  Verify it calls `.as_u64()` (rejects negatives at the JSON level) then
  `usize::try_from(v)` (rejects values too large for the platform).

- [ ] **Scan for `as usize` in the artifact decode path:**
  ```bash
  grep -n 'as usize' Rust/src/bytecode/artifact.rs Rust/src/artifact_io.rs
  ```
  Expected: zero matches. Any match is a GAP.

- [ ] **Scan for `as usize` in the JIT operand conversion path:**
  ```bash
  grep -n 'as usize' Rust/src/jit/op.rs Rust/src/jit/chunk.rs
  ```
  Expected: zero matches (all conversions go through `jit_operand` / `checked_non_negative_usize`).

- [ ] **Record finding H** in the findings report.

---

### Produce Audit Report

- [ ] **Write findings to `docs/audit-findings.md`** in this format:

  ```
  ## Audit Findings

  A. FFI Panic Boundary: OK ffi.rs:107-147 — double catch_unwind, fallback_response safe
  B. C String Ownership: OK tinyone.h:1-147 — all params documented, nullable marked
  C. Panic Paths: OK — no bare .unwrap() in public paths; jit_operand returns Result
  D. Verified Execution: OK — BytecodeVerifier::verify called in every public run/JIT path
  E. Artifact Limits: OK artifact.rs:8-24 — all limits present, reject before alloc
  F. Verifier Limits: OK verifier.rs:5-8 — MAX_VERIFIER_STEPS + MAX_STACK_DEPTH enforced
  G. FS Budget: OK stdlib.rs:793-875 — metadata check before open, entry+byte limits
  H. usize Conversion: OK artifact.rs:250-259 — as_u64 + try_from; no bare `as usize`

  GAPS: [list any gaps, or "none"]
  ```

  If any GAP is found, describe the exact file:line and what is missing.
  Pass findings to Task 3 (Implementation) and Task 4 (Test).

---

## Task 2 — Adversarial Agent  *(parallel with Task 1)*

**What this task does:** Probes the existing implementation with hostile inputs. Every
probe should be a reproducible test or command. Reports anything that panics, produces
unexpected output, or is still exploitable.

**All probes run from `Rust/`:**
```bash
cd /path/to/TinyOne/Rust
```

---

### Probe Set 1 — FFI Null/Invalid Pointers

- [ ] **Verify the null-pointer tests already in `abi_api_soundness.rs` cover all
  six FFI entry points.** Run:
  ```bash
  cargo test --test abi_api_soundness ffi_null_pointers_return_valid_json_errors -- --nocapture
  ```
  Expected: `test ... ok`

- [ ] **Verify the free-null test:**
  ```bash
  cargo test --test abi_api_soundness ffi_free_string_accepts_null -- --nocapture
  ```
  Expected: `test ... ok`

- [ ] **Probe: all-nulls to `tinyone_run_source_json`** (already covered by the null test,
  confirm `source pointer was null` is the error, not a panic about mode).

---

### Probe Set 2 — Hostile Artifacts via `Program::from_artifact`

- [ ] **Run:**
  ```bash
  cargo test --test abi_api_soundness artifact_rejects_huge_counts_before_accepting_program -- --nocapture
  ```
  Expected: `test ... ok`

- [ ] **Craft artifact with `slot_count = u64::MAX` serialized as a large integer:**
  Add a temporary inline test or use `cargo test` with the existing infrastructure:
  ```bash
  cargo test --test abi_api_soundness artifact_rejects_invalid_integer_fields_and_file_size -- --nocapture
  ```
  Expected: `test ... ok` (confirms `expect_usize` rejects out-of-range values)

- [ ] **Craft artifact where main code list is `[null, null, null, …]`** (already tested
  by `artifact_rejects_invalid_integer_fields_and_file_size` via the missing-arg test).
  Confirm `Instruction artifact must be an object` is the error message.
  ```bash
  cargo test --test abi_api_soundness -- --nocapture 2>&1 | grep -E "ok|FAILED"
  ```

---

### Probe Set 3 — Verifier Stress

- [ ] **Run the dense jump graph and stack bomb tests:**
  ```bash
  cargo test --test abi_api_soundness verifier -- --nocapture
  ```
  Expected: all three verifier tests pass (`verifier_handles_dense_jump_graph_without_path_explosion`,
  `verifier_rejects_oversized_dense_jump_graph_before_stress`,
  `verifier_rejects_stack_depth_bomb`).

- [ ] **Probe: backward-jump loop that visits the same node 10M+ times.** Craft manually:
  Write a short Rust snippet in a temporary test. `minimal_program()` is defined at
  `Rust/tests/abi_api_soundness.rs:123-134` and returns:
  ```rust
  Program {
      code: vec![Instr::new(Op::Halt, 0, 0)],
      slot_count: 0,
      names: Vec::new(),
      functions: Vec::new(),
      strings: Vec::new(),
      structs: Vec::new(),
      fields: Vec::new(),
      modules: Vec::new(),
  }
  ```
  Use the same pattern:
  ```rust
  // Tight infinite-loop-looking graph: PUSH_INT 0, JUMP_IF_ZERO 0 (jumps back to start)
  // The verifier should detect the depth conflict at PC 0 and terminate cleanly.
  let code = vec![
      Instr::new(Op::PushInt, 0, 0),    // PC 0 — pushes, depth 1
      Instr::new(Op::JumpIfZero, 0, 0), // PC 1 — pops, jumps to PC 0
      Instr::new(Op::Halt, 0, 0),        // PC 2 — never reached
  ];
  let prog = Program { code, slot_count: 0, names: vec![], functions: vec![],
      strings: vec![], structs: vec![], fields: vec![], modules: vec![] };
  // This must terminate immediately with a depth-mismatch error, not run for 10M steps.
  let result = BytecodeVerifier::verify(&prog);
  assert!(result.is_err(), "infinite loop graph must be rejected");
  ```
  Run this probe inline or as a temporary `#[test]` in `abi_api_soundness.rs`, then
  **remove** it before committing if it is already covered by the existing tests.

  If it panics or hangs for more than 1 second, that is a GAP — record it.

---

### Probe Set 4 — Filesystem Budget

- [ ] **Run both FS tests:**
  ```bash
  cargo test --test abi_api_soundness fs_ -- --nocapture
  ```
  Expected: both `fs_read_rejects_oversized_file_before_buffer_allocation` and
  `fs_list_dir_limit_returns_error_not_panic` pass.

- [ ] **Probe: `fs_list_dir` with entries whose names total > 1 MiB.** The existing
  test uses 5,000 × 254-byte names ≈ 1.27 MiB. Verify the test actually hits the
  byte-budget path, not just the count path, by checking the error message:
  ```bash
  cargo test --test abi_api_soundness fs_list_dir_limit_returns_error_not_panic -- --nocapture 2>&1
  ```
  The error should contain `"limit"` (either count or bytes). Confirm at least one of
  the two budget checks triggers.

---

### Probe Set 5 — JIT with Invalid Program

- [ ] **Run:**
  ```bash
  cargo test --test abi_api_soundness jit_compile_rejects_invalid_unverified_program -- --nocapture
  ```
  Expected: `test ... ok`

- [ ] **Probe: `JitProgram::compile` on a program whose code list is empty** (no `HALT`).
  Write inline (expand `minimal_program()` manually as above):
  ```rust
  let empty_prog = Program { code: vec![], slot_count: 0, names: vec![],
      functions: vec![], strings: vec![], structs: vec![], fields: vec![], modules: vec![] };
  let result = JitProgram::compile(&empty_prog);
  assert!(result.is_err(), "empty main chunk must be rejected by verifier");
  ```
  If this panics rather than returning `Err`, that is a GAP.

---

### Probe Set 6 — Invalid Char Index

- [ ] **Run:**
  ```bash
  cargo test --test abi_api_soundness invalid_char_index_path_returns_error_not_panic -- --nocapture
  ```
  Expected: `test ... ok`

---

### Probe Set 7 — Invalid Mode

- [ ] **Run:**
  ```bash
  cargo test --test abi_api_soundness ffi_invalid_mode_returns_structured_error -- --nocapture
  ```
  Expected: `test ... ok`. Confirm the error kind is `"runtime"` and contains
  `"Unsupported mode"`.

---

### Probe Set 8 — Invalid UTF-8 Source

- [ ] **Run:**
  ```bash
  cargo test --test abi_api_soundness ffi_invalid_unicode_scalar_source_returns_error -- --nocapture
  ```
  Expected: `test ... ok`.

---

### Produce Adversarial Report

- [ ] **Write a short report to `docs/adversarial-findings.md`** listing each probe result:
  ```
  ## Adversarial Findings

  Probe 1 (FFI nulls):        PASS — all 7 entry points return JSON errors
  Probe 2 (Hostile artifacts): PASS — huge counts, bad types, sparse file all rejected
  Probe 3 (Verifier stress):   PASS — dense graphs and stack bombs terminate cleanly
  Probe 4 (FS budget):         PASS — both count and byte limits enforced
  Probe 5 (JIT invalid):       PASS — Err returned, cache stays empty
  Probe 6 (Char index):        PASS — Err returned, no panic
  Probe 7 (Invalid mode):      PASS — runtime error with correct kind
  Probe 8 (Invalid UTF-8):     PASS — compile error returned

  EXPLOITS FOUND: [list, or "none"]
  ```

  If any probe produces a panic, wrong output, or an exploit, mark it as FAIL and
  describe the exact trigger. Pass to Task 3 (Implementation) and Task 4 (Test).

---

## Task 3 — Implementation Agent

**Input:** Audit findings (Task 1) and Adversarial findings (Task 2).

**What this task does:** Applies the minimal fix for each identified GAP. If no gaps
were found, this task records "no changes required" and proceeds.

**Constraint:** Do not redesign; do not add abstractions beyond what a gap requires;
do not add comments; do not change passing tests.

---

- [ ] **Read the Audit findings and Adversarial findings.**

- [ ] **If no gaps were found in either report**, write `No implementation changes required.`
  to stdout and skip to Task 4.

- [ ] **For each GAP identified:**

  **A-gap (FFI panic boundary):**
  If any `extern "C"` function can reach a panic outside `catch_unwind`:
  - Wrap the offending call in `catch_unwind(AssertUnwindSafe(...))`.
  - Or move it inside the `respond(|| { ... })` closure already guarded.

  **C-gap (Bare `.unwrap()` in public path):**
  For each bare `.unwrap()` found:
  - Replace with `.map_err(|_| TinyOneError::runtime("descriptive message"))?` or
    `.ok_or_else(|| TinyOneError::runtime("message"))?`.
  - If the unwrap is on an infallible value (e.g., known-valid formatting), replace
    with `unwrap_or_else(|_| unreachable!())` only if truly impossible to fail, and
    add a comment explaining why.

  **D-gap (Missing verifier call):**
  If a public `run_*` function or `VM::new` does not call `BytecodeVerifier::verify`:
  - Add `BytecodeVerifier::verify(program)?;` as the first statement.

  **E-gap (Limit check after allocation):**
  If `from_artifact` allocates before checking a limit:
  - Move the `reject_over_limit(...)` call above the `collect()` or `Vec::with_capacity`.

  **F-gap (Missing verifier work cap):**
  If the BFS loop does not check `steps`:
  - Add `steps += 1; if steps > MAX_VERIFIER_STEPS { return Err(...); }` inside the loop.

  **G-gap (FS read without pre-check):**
  If `b_fs_read` reads first and checks size after:
  - Add the `metadata().len() > MAX_BUFFER_BYTES` check before `File::open`.

  **H-gap (Bare `as usize` from serialized integer):**
  If artifact decode uses `i64 as usize` or similar:
  - Replace with `usize::try_from(v).map_err(|_| TinyOneError::compile(...))?`.

- [ ] **Run the full test suite to confirm no regression:**
  ```bash
  cd Rust && cargo test 2>&1 | tail -5
  ```
  Expected: all test suites report `ok`.

- [ ] **Commit each gap fix separately:**
  ```bash
  git add Rust/src/<affected-file>.rs
  git commit -m "fix(soundness): <one-line description of gap closed>

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Task 4 — Test Agent

**Input:** Audit findings (Task 1) and Adversarial findings (Task 2).

**What this task does:** Adds regression tests for every gap that was fixed in Task 3.
If no gaps were fixed, verifies that the 15 existing tests cover all required scenarios
and confirms the full suite passes.

---

- [ ] **Run the full soundness test suite:**
  ```bash
  cd Rust && cargo test --test abi_api_soundness 2>&1
  ```
  Expected output:
  ```
  running 15 tests
  test ... ok
  ...
  test result: ok. 15 passed; 0 failed
  ```

- [ ] **For each gap fixed in Task 3, write a regression test that:**
  1. Was **not** present before Task 3's fix.
  2. Fails on the pre-fix code (verify by reverting locally, running, then re-applying).
  3. Passes after the fix.

  Example pattern for a new gap test — add to `Rust/tests/abi_api_soundness.rs`:
  ```rust
  #[test]
  fn <snake_case_description_of_gap>() {
      // Arrange: construct the hostile input that exposed the gap.
      // ...

      // Act + Assert: the hostile input must produce Err, not panic.
      let result = panic::catch_unwind(|| { /* call the fixed API */ });
      let inner = result.expect("<what the panic would have said>");
      expect_error_contains(inner, "<error substring>");
  }
  ```

- [ ] **If no gaps were fixed, verify the 15 existing tests match the required test
  list from the spec.** Required tests:

  | # | Test function name | Spec requirement |
  |---|-------------------|-----------------|
  | 1 | `ffi_success_responses_are_valid_json` | FFI valid call returns valid JSON |
  | 2 | `ffi_null_pointers_return_valid_json_errors` | FFI null pointer → JSON error |
  | 3 | `ffi_free_string_accepts_null` | `tinyone_free_string(NULL)` is safe |
  | 4 | `ffi_invalid_mode_returns_structured_error` | Invalid mode → structured error |
  | 5 | `ffi_invalid_unicode_scalar_source_returns_error` | Invalid UTF-8 → error |
  | 6 | `artifact_rejects_huge_counts_before_accepting_program` | Huge slot/string/fn/code counts → Err before alloc |
  | 7 | `artifact_rejects_invalid_integer_fields_and_file_size` | Large int fields + oversized file → Err |
  | 8 | `verifier_handles_dense_jump_graph_without_path_explosion` | Dense graph → bounded time |
  | 9 | `verifier_rejects_oversized_dense_jump_graph_before_stress` | Over-limit graph → `"total instruction count"` error |
  | 10 | `verifier_rejects_stack_depth_bomb` | Stack bomb → `"stack depth"` error |
  | 11 | `fs_read_rejects_oversized_file_before_buffer_allocation` | Oversized file → `"file size"` error |
  | 12 | `fs_list_dir_limit_returns_error_not_panic` | Over-limit dir → `"limit"` error, no panic |
  | 13 | `jit_compile_rejects_invalid_unverified_program` | Invalid program → `"Verifier"` error, cache stays empty |
  | 14 | `invalid_char_index_path_returns_error_not_panic` | Huge char index → `"index"` error, no panic |
  | 15 | `public_safe_rust_paths_verify_untrusted_programs` | All public run/JIT APIs reject unverified invalid program |

  For each entry, confirm the function exists in `Rust/tests/abi_api_soundness.rs`:
  ```bash
  grep -n 'fn ffi_\|fn artifact_\|fn verifier_\|fn fs_\|fn jit_\|fn invalid_\|fn public_' \
      Rust/tests/abi_api_soundness.rs
  ```
  Expected: exactly 15 `fn` lines matching the table above.

- [ ] **Run the full test suite one final time:**
  ```bash
  cd Rust && cargo test 2>&1 | grep -E "^test result"
  ```
  Every line must say `ok`.

- [ ] **Commit any new tests:**
  ```bash
  git add Rust/tests/abi_api_soundness.rs
  git commit -m "test(soundness): add regression tests for gaps found in audit

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```
  (Skip commit if no new tests were added.)

---

## Task 5 — Review Agent

**What this task does:** Reads the final state of every modified file and produces a
pass/fail verdict for each acceptance criterion.

---

- [ ] **Read these files in full:**
  - `Rust/src/ffi.rs`
  - `tinyone.h`
  - `Rust/src/bytecode/verifier.rs`
  - `Rust/src/bytecode/artifact.rs`
  - `Rust/src/bytecode/program.rs`
  - `Rust/src/runner.rs`
  - `Rust/src/runtime/stdlib.rs`
  - `Rust/src/runtime/vm.rs`
  - `Rust/src/jit/program.rs`
  - `Rust/src/jit/cache.rs`
  - `Rust/src/jit/op.rs`
  - `Rust/tests/abi_api_soundness.rs`

- [ ] **Check acceptance criterion 1 — no C ABI path can unwind Rust panics:**
  Verify that every `#[unsafe(no_mangle)] pub extern "C" fn` in `ffi.rs` delegates
  to `respond(...)` which double-wraps with `catch_unwind`. If any `extern "C"` function
  does *not* go through `respond`, mark FAIL with the line number.

- [ ] **Check acceptance criterion 2 — public safe Rust cannot run unverified programs:**
  In `runner.rs`, `runtime/vm.rs`, `jit/program.rs`, `jit/cache.rs`:
  - `run_program`, `run_program_with_env`, `run_program_report` → `BytecodeVerifier::verify` before dispatch
  - `VM::new` → `BytecodeVerifier::verify` as first statement
  - `JitProgram::compile` → `BytecodeVerifier::verify` as first statement
  - `JitCache::compile`, `run_program`, `run_program_report`, `run_program_with_env` → `BytecodeVerifier::verify`
  - `write_jit_listing` → calls `JitProgram::compile` which calls verifier

  Mark PASS or FAIL for each.

- [ ] **Check acceptance criterion 3 — hostile artifacts fail before dangerous allocation:**
  In `artifact.rs` `from_artifact`:
  - `reject_over_limit` for `functions`, `slot_count`, `strings`, `fields`, `structs`
    must appear before any `.collect::<Result<Vec<_>>>()`.
  - `total_code_ops` accumulation must use `checked_add` and `reject_over_limit`.
  - No `Vec::with_capacity(untrusted_value)` before a limit check.

  Mark PASS or FAIL.

- [ ] **Check acceptance criterion 4 — filesystem builtins obey host budgets:**
  In `stdlib.rs`:
  - `b_fs_read`: `metadata().len() > MAX_BUFFER_BYTES` check before `File::open`.
  - `b_fs_list_dir`: entry count and name bytes both limited; `checked_add` for bytes.

  Mark PASS or FAIL.

- [ ] **Check acceptance criterion 5 — regression tests pass:**
  ```bash
  cd Rust && cargo test 2>&1 | grep -E "^test result"
  ```
  All lines must say `ok`. Mark PASS or FAIL.

- [ ] **Check for remaining undocumented contracts:**
  - Does `tinyone.h` document the JSON response schema? (It should have the
    `{"ok":true,"value":{...}}` and error shapes documented.)
  - Does the header state `ABI STATUS: UNSTABLE`?
  - Is `inputs_json` nullability documented in every affected signature?

- [ ] **Record any remaining risks** — issues that are known but out of scope for
  Phase 1 (e.g., "Program struct fields are public and can be mutated to bypass
  verifier after construction — recommend making fields private in Phase 2").

- [ ] **Produce the Review verdict:**
  ```
  ## Review Verdict

  AC1 (No C ABI panic unwind):     PASS / FAIL
  AC2 (Verified execution boundary): PASS / FAIL
  AC3 (Hostile artifacts rejected):  PASS / FAIL
  AC4 (FS budget enforced):          PASS / FAIL
  AC5 (Regression tests pass):       PASS / FAIL

  Remaining risks:
  - [list or "none identified"]

  Overall Phase 1 verdict: PASS / FAIL
  ```

---

## Task 6 — Release-Gate Report

**What this task does:** Aggregates findings from all five agents into the final
release-gate report. Write to `docs/release-gate-phase1.md` and commit.

---

- [ ] **Collect:**
  - Audit findings report (Task 1 output)
  - Adversarial findings report (Task 2 output)
  - List of files changed in Task 3 (or "none")
  - List of tests added in Task 4 (or "none")
  - Review verdict (Task 5 output)

- [ ] **Write `docs/release-gate-phase1.md`:**

  ```markdown
  # TinyOne Phase 1 Release-Gate Report

  **Date:** 2026-05-18
  **Branch:** main
  **Test suite:** `cargo test` — all suites must pass

  ## Fixed Defects

  | ID | Area | Status | File:Line |
  |----|------|--------|-----------|
  | A | FFI panic boundary | [FIXED/ALREADY OK] | ffi.rs:107-147 |
  | B | C string ownership contract | [FIXED/ALREADY OK] | tinyone.h |
  | C | Panic-producing paths | [FIXED/ALREADY OK] | jit/op.rs, runtime/stdlib.rs |
  | D | Verified execution boundary | [FIXED/ALREADY OK] | runner.rs, vm.rs, jit/program.rs, jit/cache.rs |
  | E | Artifact resource limits | [FIXED/ALREADY OK] | bytecode/artifact.rs |
  | F | Verifier work and stack limits | [FIXED/ALREADY OK] | bytecode/verifier.rs |
  | G | Host filesystem budget | [FIXED/ALREADY OK] | runtime/stdlib.rs |
  | H | 32-bit/usize conversion safety | [FIXED/ALREADY OK] | bytecode/artifact.rs, jit/op.rs |

  ## Files Changed

  [List from Task 3, or "No source files changed — all fixes were already present."]

  ## Tests Added

  [List from Task 4, or "No new tests added — all 15 required tests were already present and passing."]

  ## Remaining Known Risks

  [List from Task 5 Review, or "None identified in Phase 1 scope."]

  ## Acceptance Criteria Verdict

  | Criterion | Verdict |
  |-----------|---------|
  | No C ABI path can unwind Rust panics | [PASS/FAIL] |
  | Public safe Rust cannot run unverified programs | [PASS/FAIL] |
  | Hostile artifacts fail before dangerous allocation | [PASS/FAIL] |
  | Filesystem builtins obey host budgets | [PASS/FAIL] |
  | Regression tests pass | [PASS/FAIL] |

  ## Phase 1 Verdict: [PASS / FAIL]
  ```

- [ ] **Commit the report:**
  ```bash
  git add docs/release-gate-phase1.md
  git commit -m "docs: add Phase 1 release-gate report

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

- [ ] **Run the full test suite one final time and include the output in the report:**
  ```bash
  cd Rust && cargo test 2>&1 | grep -E "^test result|FAILED"
  ```
  Expected: all lines say `ok`, zero `FAILED`.
