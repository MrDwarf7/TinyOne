# TinyOne ABI/API Soundness Gate — Design Spec

**Date:** 2026-05-18
**Status:** Approved

---

## Goal

Fix all release-blocking ABI/API soundness defects in the TinyOne repository before
any stability claim is made. Scope is limited to soundness and safety; no new features.

---

## Current State

All 15 required soundness tests in `Rust/tests/abi_api_soundness.rs` pass on the
working tree as of this design. The working tree also contains a complete `tinyone.h`
header. Modified source files cover all eight defect areas (A–H). No commit has been
made yet.

---

## Required Defect Areas (A–H)

| ID | Area | Files affected |
|----|------|---------------|
| A | FFI panic boundary | `Rust/src/ffi.rs` |
| B | C string ownership contract | `tinyone.h` |
| C | Panic-producing unwrap/expect/direct-index paths | `Rust/src/runtime/`, `Rust/src/jit/` |
| D | Verified execution boundary | `Rust/src/bytecode/program.rs`, `Rust/src/runner.rs`, `Rust/src/jit/` |
| E | Artifact resource limits | `Rust/src/bytecode/artifact.rs` |
| F | Verifier work and stack limits | `Rust/src/bytecode/verifier.rs` |
| G | Host filesystem budget enforcement | `Rust/src/runtime/stdlib.rs` |
| H | 32-bit / usize conversion safety | `Rust/src/bytecode/artifact.rs` |

---

## Execution Design — 5-Agent Pipeline

### Phase 1 — Parallel (read-only)

**Agent 1: Audit**
- Re-scan each A–H fix for completeness and correctness.
- Verify exact file/line evidence for each fix.
- Identify any gap (missing fix, incomplete wiring, wrong boundary).
- Output: structured findings list with file:line references.

**Agent 2: Adversarial**
- Probe the implementation with hostile inputs independently of Audit.
- Test vectors: malformed artifacts, huge counts, null/invalid pointers (where testable),
  crafted jump graphs, stack bombs, heap pressure, invalid JIT programs, oversized
  filesystem inputs, bad modes, invalid char codepoints.
- Output: list of anything still exploitable or behaviorally wrong.

### Phase 2 — Sequential (fix gaps)

**Agent 3: Implementation**
- Receives Phase 1 findings.
- Applies the minimal set of changes to close identified gaps.
- No redesign; correctness-first; no AI placeholders.

**Agent 4: Test**
- Receives Phase 1 findings.
- Adds regression tests for every gap not already covered.
- Tests must fail before the fix and pass after.
- Includes Rust and C/FFI tests where applicable.

### Phase 3 — Sequential (final review)

**Agent 5: Review**
- Inspects the complete patch.
- Checks: incomplete error handling, undocumented contracts, panic paths,
  unchecked allocations, public API unsoundness.
- Produces pass/fail verdict per requirement.

### Data Flow

```
[Audit] ──┐
           ├──→ [Implementation] → [Test] ──→ [Review] → Release-Gate Report
[Adv]  ────┘
```

---

## Acceptance Criteria

- No known public C ABI path can unwind Rust panics across FFI.
- Public safe Rust execution cannot run/JIT unverified invalid programs without verification.
- Hostile artifacts fail cleanly before dangerous allocation.
- Filesystem builtins obey host budgets.
- All regression tests pass.

---

## Deliverables

1. Patch implementing fixes for any identified gaps.
2. Regression tests for every fixed issue.
3. Release-gate report: fixed defects, files changed, tests added, remaining risks,
   Phase 1 pass/fail verdict.

---

## Constraints

- Do not hide failures with broad catch-all behavior.
- Do not weaken tests to pass.
- Do not introduce AI-generated placeholders.
- Do not remove functionality unless unsafe and explicitly justified.
- Do not claim stable ABI/API in docs.
- Keep changes minimal and reviewable.
