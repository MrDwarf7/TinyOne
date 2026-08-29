---
title: TinyLang Documentation
---

**Design status:** TinyLang v1 is an active release line. The v1 ABI is
frozen for the entire v1 lifecycle. Syntax, FFI details beyond the
frozen ABI, allocator, VM, JIT, memory design, and semantics may still
evolve. TinyLang v2 is expected to make major changes, including a new
language-boundary design. These documents describe the current line and
are not a promise of perpetual compatibility.

Send comments, concerns, and questions to the [TinyLang community
forum](https://tl.404connernotfound.dev).

# Language Users

Writing TinyLang programs:

- [Syntax: Types](syntax/types.md) \-- int, string, array, struct,
  buffer, cell, pointer, and null.
- [Syntax: Statements](syntax/statements.md) \-- let, if, while, fn,
  struct, import, and more.
- [Syntax: Expressions](syntax/expressions.md) \-- operators and
  precedence.
- [Syntax: Modules](syntax/modules.md) \-- import/export and
  `tinyone.json` manifests.
- [Standard Library](stdlib.md) \-- all Phase-1 and Phase-2 builtins.
- [Examples](examples.md) \-- runnable programs by feature.
- [CLI Reference](cli.md) \-- flags and workflow examples.

# Integrators and Embedders

Embedding the TinyOne implementation in a host application:

- [C Integration Guide](ffi/c-integration.md) \-- building, linking,
  entry points, and ownership.
- [Rust API Reference](ffi/rust-api.md) \-- `compile_source`,
  `run_source`, `JitCache`, and more.
- [ABI Contract](abi/contract.md) \-- panic boundary, null safety,
  thread safety, and response envelope.
- [ABI Schemas](abi/schemas.md) \-- exact JSON response schemas per
  endpoint.
- [ABI Versioning](abi/versioning.md) \-- frozen v1 policy and the v2
  compatibility boundary.

# Contributors

Working on the TinyOne runtime implementation:

- [Architecture](architecture.md) \-- pipeline overview, module map,
  stage details, and key invariants.
- [Bytecode Reference](bytecode.md) \-- opcode table, artifact format,
  verifier rules, and JIT tier.
- [VM and JIT Operation](vm.md) \-- dispatch loop, frame model,
  quickening lifecycle, and `JitCache`.
- [Performance Workflow](performance.md) \-- Windows/Linux counters,
  workloads, baselines, and priorities.
- [Memory Model](memory-model.rst) \-- heap slab, generation tags,
  ownership rules, and resource limits.
- [Contributing Guide](contributing.md) \-- build, test, adding
  features, builtins, and stdlib modules.
- [v2 Roadmap](v2-roadmap.rst) \-- current language-generation
  commitments and implementation tracks.
- [Community Forum](Community_Forum.md) \-- request, proposal, and
  implementation-notice process.

# All Documents

TinyLang\'s v1 ABI is frozen for the v1 lifecycle. TinyLang does not
promise to keep old implementations or all historical language versions
available forever.

- [abi/contract.md](abi/contract.md) \-- Runtime invariants: panic
  boundary, null safety, ownership, and thread safety.
- [abi/index.rst](abi/index.rst) \-- ABI area navigation.
- [abi/schemas.md](abi/schemas.md) \-- JSON response schemas per entry
  point.
- [abi/versioning.md](abi/versioning.md) \-- Frozen v1 policy and v2
  compatibility boundary.
- [adversarial-findings.rst](adversarial-findings.rst) \-- Adversarial
  test findings from Phase 1 review.
- [architecture.md](architecture.md) \-- Pipeline, module map, stage
  details, and key invariants.
- [audit-findings.rst](audit-findings.rst) \-- Audit findings from Phase
  1 review.
- [bytecode.md](bytecode.md) \-- Opcode table, artifact format, verifier
  rules, and JIT adaptive tier.
- [cli.md](cli.md) \-- CLI flags and workflow examples.
- [contributing.md](contributing.md) \-- Build, test, adding language
  features, builtins, and stdlib modules.
- [examples.md](examples.md) \-- Runnable TinyLang programs by feature.
- [ffi/c-integration.md](ffi/c-integration.md) \-- C embedding guide:
  build, link, entry points, ownership, and threading.
- [ffi/index.rst](ffi/index.rst) \-- FFI area navigation.
- [ffi/rust-api.md](ffi/rust-api.md) \-- Rust crate public API: compile,
  run, JIT, artifacts, and verification.
- [memory-model.rst](memory-model.rst) \-- Heap slab, `HeapRef`
  generation validation, ownership rules, and limits.
- [performance.md](performance.md) \-- Cross-platform benchmark
  workflow, workload map, and optimization priorities.
- [planned/tinylang_ffi_v2.md](planned/tinylang_ffi_v2.md) \-- Typed C
  ABI proposal for v2.
- [stdlib.md](stdlib.md) \-- Phase-1 core builtins and Phase-2 stdlib
  bridge reference.
- [syntax/expressions.md](syntax/expressions.md) \-- Operators,
  precedence table, arithmetic, comparisons, and unsafe gate.
- [syntax/index.rst](syntax/index.rst) \-- Syntax area navigation.
- [syntax/modules.md](syntax/modules.md) \-- Import/export, path
  resolution, `tinyone.json` manifest, and worked example.
- [syntax/statements.md](syntax/statements.md) \-- Every statement form
  with syntax, semantics, and examples.
- [syntax/types.md](syntax/types.md) \-- All value types: creation,
  mutation, errors, and ownership.
- [v2-roadmap.rst](v2-roadmap.rst) \-- v2 language-generation
  commitments and implementation tracks.
- [vm.md](vm.md) \-- VM dispatch, frame model, JIT compilation, hot-loop
  quickening, and `JitCache`.
