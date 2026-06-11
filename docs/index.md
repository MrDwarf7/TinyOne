# TinyLang Documentation

## Language Users

Writing TinyLang programs:

- [Syntax: Types](syntax/types.md) — int, string, array, struct, buffer, cell, pointer, null
- [Syntax: Statements](syntax/statements.md) — let, if, while, fn, struct, import, and more
- [Syntax: Expressions](syntax/expressions.md) — operators and precedence
- [Syntax: Modules](syntax/modules.md) — import/export, tinyone.json manifests
- [Standard Library](stdlib.md) — all Phase-1 and Phase-2 builtins
- [Examples](examples.md) — runnable programs by feature
- [CLI Reference](cli.md) — flags and workflow examples

## Integrators and Embedders

Embedding the TinyOne implementation in a host application:

- [C Integration Guide](ffi/c-integration.md) — building, linking, entry points, ownership
- [Rust API Reference](ffi/rust-api.md) — compile_source, run_source, JitCache, and more
- [ABI Contract](abi/contract.md) — panic boundary, null safety, thread safety, response envelope
- [ABI Schemas](abi/schemas.md) — exact JSON response schemas per endpoint
- [ABI Versioning](abi/versioning.md) — breaking-change policy, stability status, v1 plan

## Contributors

Working on the TinyOne runtime implementation:

- [Architecture](architecture.md) — pipeline overview, module map, stage details, key invariants
- [Bytecode Reference](bytecode.md) — opcode table, artifact format, verifier rules, JIT tier
- [VM and JIT Operation](vm.md) — dispatch loop, frame model, quickening lifecycle, JitCache
- [Memory Model](memory-model.md) — heap slab, generation tags, ownership rules, resource limits
- [Contributing Guide](contributing.md) — build, test, adding features, builtins, and stdlib modules
- [v1 Roadmap](v1-roadmap.md) — work required before the stable ABI release

---

## All Documents

| File | Description |
| --- | --- |
| [`abi/contract.md`](abi/contract.md) | Runtime invariants: panic boundary, null safety, ownership, thread safety |
| [`abi/index.md`](abi/index.md) | ABI area navigation |
| [`abi/schemas.md`](abi/schemas.md) | JSON response schemas per entry point |
| [`abi/versioning.md`](abi/versioning.md) | Breaking-change policy and v1 stability plan |
| [`adversarial-findings.md`](adversarial-findings.md) | Adversarial test findings from Phase 1 review |
| [`architecture.md`](architecture.md) | Pipeline, module map, stage details, key invariants |
| [`audit-findings.md`](audit-findings.md) | Audit findings from Phase 1 review |
| [`bytecode.md`](bytecode.md) | Opcode table, artifact format, verifier rules, JIT adaptive tier |
| [`cli.md`](cli.md) | CLI flags and workflow examples |
| [`contributing.md`](contributing.md) | Build, test, adding language features, builtins, stdlib modules |
| [`examples.md`](examples.md) | Runnable TinyLang programs by feature |
| [`ffi/c-integration.md`](ffi/c-integration.md) | C embedding guide: build, link, entry points, ownership, threading |
| [`ffi/index.md`](ffi/index.md) | FFI area navigation |
| [`ffi/rust-api.md`](ffi/rust-api.md) | Rust crate public API: compile, run, JIT, artifacts, verification |
| [`memory-model.md`](memory-model.md) | Heap slab, HeapRef generation validation, ownership rules, limits |
| [`release-gate-phase1.md`](release-gate-phase1.md) | Phase 1 soundness gate report |
| [`stdlib.md`](stdlib.md) | Phase-1 core builtins and Phase-2 stdlib bridge reference |
| [`syntax/expressions.md`](syntax/expressions.md) | Operators, precedence table, arithmetic, comparisons, unsafe gate |
| [`syntax/index.md`](syntax/index.md) | Syntax area navigation |
| [`syntax/modules.md`](syntax/modules.md) | Import/export, path resolution, tinyone.json manifest, worked example |
| [`syntax/statements.md`](syntax/statements.md) | Every statement form with syntax, semantics, and examples |
| [`syntax/types.md`](syntax/types.md) | All value types: creation, mutation, errors, ownership |
| [`v1-roadmap.md`](v1-roadmap.md) | Blocking and non-blocking work items before v1 ABI stability |
| [`vm.md`](vm.md) | VM dispatch, frame model, JIT compilation, hot-loop quickening, JitCache |
