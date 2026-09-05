[![Crates.io](https://img.shields.io/crates/v/tinylang.svg)](https://crates.io/crates/tinylang)
[![Crates.io](https://img.shields.io/crates/d/tinylang.svg)](https://crates.io/crates/tinylang)
[![docs.rs](https://docs.rs/tinylang/badge.svg)](https://docs.rs/tinylang)
[![CI](https://github.com/ConnerAdamsMaine/TinyOne/actions/workflows/test.yml/badge.svg)](https://github.com/ConnerAdamsMaine/TinyOne/actions/workflows/test.yml)
[![License](https://img.shields.io/badge/license-custom-blue.svg)](#license)

TinyOne is the v1 release generation of TinyLang, a portable systems
programming language designed around a compact VM, bounded runtime
assumptions, a small reasoning surface, and the freedom to discover its
own best design.

TinyLang is not tiny because its syntax is minimal. It is tiny where
systems usually become expensive: runtime footprint, allocator design,
host assumptions, VM behavior, platform dependencies, and implementation
burden.

The current Rust implementation includes a lexer, compiler, bytecode
optimizer, verifier, portable VM, heap/runtime model, bytecode artifact
support, adaptive execution support, host integration surfaces, CLI
tooling, and early allocator-integration scaffolding.

Current crate version: `1.5.1` (the implementation is now managed as the
public v1 release line while language work proceeds internally under
v2).

The current Rust crate lives in `crates/tinyone_core/` (crate name
`tinylang`) in this checkout. TinyLang is the durable language identity
and TinyOne is its v1 implementation generation. TinyLang v2 is the
active internal language roadmap: v2 features are developed on the v1
release foundation without creating a separate user-facing language.

> **Repository history note:** Multiple recent failed rebases, and
> subsequent rebase fixes, ruined the commit history. This repository has
> nevertheless been in active development for almost eight months.
> **TinyLang v2 Development Update**
>
> With the successful v1 release line locked in, active development has
> shifted to **TinyLang v2**. Key architectural upgrades currently
> underway include:
>
> - **Deterministic Binary FFI:** Replacing JSON-over-C with a zero-copy,
>   cross-system binary serialization protocol.
> - **Universal Language SDKs:** First-class test crates coming for **C,
>   Python, JavaScript (Node.js), and Rust**.
> - **Unified Memory Determinism:** Deepening `Ralloc` integration across
>   all VM and runtime primitives.
>
> _The frozen v1 ABI remains stable in `crates/tinyone_core/` while v2
> features evolve internally._

---

- [General Information](#general-information)
- [What Tiny Means](#what-tiny-means)
- [Project Goals](#project-goals)
- [Build Instructions](#build-instructions)
- [Command Line](#command-line)
- [Language Overview](#language-overview)
- [Runtime and Memory Model](#runtime-and-memory-model)
- [Hard Resource Limits](#hard-resource-limits)
  - [Bytecode artifacts](#bytecode-artifacts)
  - [Artifact authority](#artifact-authority)
  - [Runtime, source, and FFI](#runtime-source-and-ffi)
- [VM and JIT](#vm-and-jit)
- [C FFI](#c-ffi)
- [Documentation](#documentation)
- [Developer Tools](#developer-tools)
- [Known Implementation Gaps](#known-implementation-gaps)
  - [Documentation status](#documentation-status)
  - [Repository and documentation drift](#repository-and-documentation-drift)
  - [Test and verification gaps](#test-and-verification-gaps)
  - [Language and runtime gaps](#language-and-runtime-gaps)
  - [Allocator boundary](#allocator-boundary)
- [Tests and Benchmarks](#tests-and-benchmarks)
- [Repository Layout](#repository-layout)
- [Release Direction](#release-direction)
- [LICENSE](#license)
- [Feedback and Community](#feedback-and-community)

<a id="general-information"></a>

# General Information

- Source crate: `crates/tinyone_core/` (crate: `tinylang`)
- C FFI header: `tinylang.h`
- Public documentation: `docs/`
- Allocator crate: `crates/tinyone_ralloc/` (crate: `tinyone_ralloc`)
- Developer tools: `tools/`
- Build/orchestration: `crates/xtask/`, `Makefile.toml`, `build/`

TinyLang is now in its v1 release line, but v1 and earlier are
deliberately discovery versions. The syntax, bytecode format, builtin
set, JSON artifacts, FFI details, allocator, VM, JIT, memory design, and
semantics are subject to change. The v1 ABI itself is frozen for the
entire v1 lifecycle. Nothing in the syntax, FFI, or allocator
documentation is a promise of permanence or perfection. These versions
exist to test theories, kill stereotypes, and proactively develop the
model we are working toward.

TinyLang v2 will be a major version jump with massive changes, including
a new language-boundary design. The ABI is being saved for that major
jump because it defines how two languages communicate. Documentation
describes current behavior and design direction, not an unchangeable
promise across v2.

TinyLang is working toward a language that leaves no excess overhead,
uses the smallest footprint it can without giving up reliability, and
proactively pushes the idea of necessary performance loss out the
window. Reaching that goal means changing the v2 ABI, FFI, allocator,
VM, JIT, and surrounding implementation model when the evidence says we
should.

<a id="what-tiny-means"></a>

# What Tiny Means

TinyLang is designed to stay small at the architectural level, not
necessarily at the syntax level.

The language may expose a broad syntax, many types, builtins, default
syntax, VM/runtime support, host interop, and a growing standard
library. The constraint is that each feature should preserve a compact
operational model. When code, syscalls, assembly, platform assumptions,
or runtime machinery can be removed without damaging reasoning or
capability, they should be removed.

The largest implementation pieces may be the VM and memory allocator,
but even those should remain compact, inspectable, and understandable.
The goal is a capable language with a compact operational core that
remains portable across systems.

<a id="project-goals"></a>

# Project Goals

TinyLang is intended to be a capable all-in-one language/runtime
implementation with a compact operational core for:

- low-level programming
- high-level integration
- explicit memory and pointer work
- VM-enforced runtime safety
- deterministic non-GC cleanup direction
- practical multithreaded workloads
- compiler, bytecode, verifier, VM, JIT, FFI, and allocator education

TinyLang does not aim to hide unsafe operations. Operations that can
affect memory, runtime state, pointer provenance, or host resources
should be explicit and checked by the runtime wherever possible.

<a id="build-instructions"></a>

# Build Instructions

Build the Rust crate and CLI executable:

    cargo build --manifest-path crates/tinyone_core/Cargo.toml

Run the repo-local CI/release gate from the repository root:

    cargo run --manifest-path crates/xtask/Cargo.toml -- release-gate

The gate checks tinyone_core, tinyone_ralloc, the xtask harness, language
fixtures, formatting, Clippy, benchmark smoke coverage, and ABI drift. It
requires `uv` for the Python tooling steps.

This creates the debug executable at `target/debug/tinylang`.
Build with `--release` when you want the optimized executable at
`target/release/tinylang`. The Windows executable name
is `tinylang.exe`. The examples below assume the executable is available on
`PATH` as `tinylang`.

Run the command-line tool directly:

    tinylang --help

Run a source file with the default adaptive JIT mode:

    tinylang program.to

Run the same source through the portable VM:

    tinylang --mode vm program.to

Compile and verify without running:

    tinylang --check program.to

Emit bytecode and JIT listings:

    tinylang --emit-bytecode program.tobc.json program.to
    tinylang --emit-bytecode program.tob program.to
    tinylang --emit-jit program.jit.txt program.to

Run a bytecode artifact:

    tinylang --run-bytecode program.tobc.json

<a id="command-line"></a>

# Command Line

The CLI supports:

    usage: tinylang [OPTIONS] [path]

    Options:
      --mode {jit,vm}       Execution mode (default: jit)
      -j, --jit             Use adaptive JIT mode
      --vm                  Use portable VM mode
      -O0, --no-optimize    Disable bytecode optimization
      -O1, --optimize       Enable bytecode optimization (default)
      --no-cache            Disable the dependency-validated disk compile cache
      --jit-threshold N     Quicken loops after N back edges (default: 8)
      --no-jit-quickening   Disable adaptive JIT quickening
      --check               Compile only, do not run
      --emit-bytecode PATH  Write JSON, or compact binary for a .tob path
      --emit-jit PATH       Write a JIT listing to PATH
      --run-bytecode PATH   Run a compiled bytecode artifact
      --input VALUE         Supply a program input value (repeatable)
      --stdin               Read input values from stdin
      --verbose             Print program metadata before running
      -h, --help            Show help

<a id="language-overview"></a>

# Language Overview

The implemented language currently includes:

- integer and string literals
- `null`
- `let` bindings
- assignment to existing variables
- expression statements
- `print`
- `if`, `else if`, and `else`
- `while`
- `break` and `continue`
- top-level `fn` declarations
- top-level `struct` declarations
- arrays
- strings as heap objects
- structs as heap objects
- pointer cells through `alloc`, `load`, `store`, and `unsafe free`
- raw pointers for objects, arrays, struct fields, buffers, and cells
- unsafe-gated pointer arithmetic, raw loads/stores, and buffer
  reads/writes
- imports with namespaces and `tinyone.json` manifest resolution
- exported module declarations
- deterministic input through `--input` and `--stdin`
- fixed-width runtime integer values for low-level memory work
- boolean operators `&&`, `||`, and `!` producing `0` or `1`

Example:

    fn add(left, right) {
      return left + right
    }

    let answer = add(40, 2)
    print answer

Current compiler constraints:

- functions and structs are top-level only
- functions must be defined before ordinary calls
- recursive self-calls are supported from inside the function body
- nested functions are rejected
- top-level executable statements are rejected inside imported modules
- imports must appear before declarations or executable statements
- functions may read earlier top-level variables, but direct assignment
  to top-level slots from inside functions is rejected
- `compile_file` supports import resolution; `compile_source` style APIs
  do not resolve imports because they compile anonymous source without a
  resolver

<a id="runtime-and-memory-model"></a>

# Runtime and Memory Model

TinyLang runs through this pipeline:

    source -> lexer -> compiler -> bytecode -> peephole optimizer -> verifier -> VM/JIT

The runtime includes:

- fixed-slot stack frames
- a generational heap slab
- heap references with generation checks
- raw pointer values with runtime provenance checks
- explicit manual deallocation through unsafe operations
- checked arithmetic and checked division
- shutdown heap draining through report APIs

TinyLang does not use a tracing garbage collector. The current runtime
uses a VM-owned heap with generation validation and explicit unsafe
deallocation. Heap payloads, VM locals/globals, and allocator-side
backing are Ralloc-owned; allocation-table bookkeeping and shutdown
remain deterministic.

<a id="hard-resource-limits"></a>

# Hard Resource Limits

TinyOne rejects inputs that exceed the following enforced v1 caps. These
are current implementation limits, not sizing recommendations; users who
generate source or bytecode should stay within them. Every loaded
artifact is verified before VM execution or JIT lowering.

<a id="bytecode-artifacts"></a>

## Bytecode artifacts

The following limits apply when loading either JSON bytecode or compact
`.tob` bytecode (and to JSON artifact text passed through the C FFI):

- artifact size: 8 MiB
- functions: 4,096; structs: 4,096; modules: 256
- code operations: 65,536 in each main/function chunk and 262,144 across
  the complete program
- slots and parameters: 65,536 per main/function chunk
- strings, field-table entries, and names in each name table: 65,536
  each
- imports per module: 4,096; function exports per module: 4,096; struct
  exports per module: 4,096
- fields per struct: 256
- operand-stack depth during bytecode verification: 65,536; verifier
  work: 10,000,000 graph steps
- metadata text: each individual name/path/string is at most 1 MiB, and
  the combined text in each string-list field is at most 1 MiB

Compact binary artifacts additionally support at most 4,096 enum
variants and 256 fields per variant. JSON artifacts do not currently
serialize enum variants, so JSON artifacts containing enum construction
are rejected by verification. The decoder applies its table and
instruction bounds before it builds each program table; the full
verifier budget check completes before execution.

<a id="artifact-authority"></a>

## Artifact authority

New JSON artifacts use schema version 2 and compact binary artifacts use
schema version 4; both record the complete root and module permission
policy. Ordinary artifact loading (including the CLI, C FFI, and
`load_artifact`) treats that policy as untrusted metadata and therefore
grants no host capabilities. An embedding application may use the
explicit trusted Rust loaders only after it has independently
authenticated the artifact bytes and accepted the recorded policy. This
prevents a supplied artifact from adding filesystem, environment,
thread, network, process, hardware, graphics, or unsafe-memory authority
on its own.

<a id="runtime-source-and-ffi"></a>

## Runtime, source, and FFI

- dynamic arrays contain at most 65,536 elements; a single buffer
  allocation is at most 1 MiB
- total live heap payload is at most 4 MiB, with at most 1,000,000 live
  heap object slots; TinyLang call nesting is limited to 16 calls
- `fs_read` reads at most 1 MiB; `fs_list_dir` returns at most 65,536
  entries and at most 1 MiB of entry-name text
- each source file, imported source file, and `tinyone.json` manifest
  read from disk is at most 1 MiB
- C-FFI source text is at most 1 MiB, file paths 32 KiB, execution modes
  16 bytes, and `inputs_json` 8 MiB (each excludes its trailing NUL).
  The sandboxed C-FFI execution APIs have a five-second deadline and a
  16 MiB response cap.
- a field-pointer or reference stored in an array, struct, or other
  fixed-width container may name a field of at most 27 UTF-8 bytes.
  Stack-resident raw pointers are not subject to this representation
  limit.

<a id="vm-and-jit"></a>

# VM and JIT

TinyLang has two execution backends:

`vm`

: Portable bytecode interpreter. It is the simpler backend and is the
main reference path for behavior checks.

`jit`

: Adaptive lowered-bytecode tier. It is not a native machine-code JIT.
It compiles verified bytecode into internal JIT ops, caches compiled
programs by fingerprint, emits inspectable listings, and quickens hot
back edges.

Both public run paths verify bytecode before execution.

<a id="c-ffi"></a>

# C FFI

The crate builds as an `rlib` and `cdylib`. The C header is
`tinylang.h`.

The FFI surface uses JSON-over-C-string entry points:

- `tinyone_lex_source_json`
- `tinyone_compile_source_json`
- `tinyone_compile_file_json`
- `tinyone_run_source_json`
- `tinyone_run_file_json`
- `tinyone_run_artifact_json`
- `tinyone_jit_listing_json`
- `tinyone_free_string`

Returned strings must be released with `tinyone_free_string`. The v1 ABI
is frozen for the entire v1 lifecycle. Check `tinyone_abi_version()`
against `TINYONE_ABI_VERSION` before using the API; expect major
incompatibilities when TinyLang v2 introduces its new language-boundary
design.

<a id="documentation"></a>

# Documentation

The main documentation tree is `docs/`:

- `docs/INDEX.md` routes readers by audience
- `docs/syntax/` describes syntax
- `docs/abi/` describes ABI contracts and versioning
- `docs/ffi/` describes C and Rust integration
- `docs/architecture.md` describes the pipeline and module map
- `docs/bytecode.md` describes opcodes, artifacts, verifier rules, and
  JIT
- `docs/memory-model.md` describes heap handles, pointer checks, and
  limits
- `docs/stdlib.md` describes builtins and stdlib bridge behavior
- `docs/v2-roadmap.md` tracks the active internal v2 language roadmap

The change-document process is defined by the TinyLang
documentation-change system:

`TLR`

: TinyLang Request. A lightweight request for a change, fix,
clarification, or improvement.

`TLP`

: TinyLang Proposal. A structured design proposal for significant
language, compiler, tooling, documentation, standard-library,
ecosystem, or governance changes.

`TLIN`

: TinyLang Implementation Notice. A release-facing or pre-release notice
explaining what is being implemented, what changed, and how users
should migrate.

The intended path is:

    TLR -> TLP -> TLIN

Small accepted changes may go directly from TLR to TLIN. Major language
changes should not skip the TLP stage.

<a id="developer-tools"></a>

# Developer Tools

`tools/hash.py`

: Stdlib-only file, tree, and manifest hashing tool for release
manifests, audit checkpoints, and source-tree integrity checks.

`tools/loc.py`

: Small line-count and audit utility for source and documentation files.

`tools/abi_manifest.py`

: ABI drift-check and symbol-manifest tool. Run
`python3 tools/abi_manifest.py check` before changing FFI entry points;
use `manifest` for a deterministic review artifact and `generate-header`
(requires `cbindgen`) to regenerate `tinylang.h`.

`tools/zip.py`

: Utility for packaging release artifacts.

Examples:

    python3 tools/hash.py README.md
    python3 tools/hash.py --tree . --format json --list-files
    python3 tools/hash.py --check manifest.json
    python3 tools/loc.py --audit --docs --json
    python3 tools/loc.py --letters

<a id="known-implementation-gaps"></a>

# Known Implementation Gaps

This section intentionally records gaps between current implementation,
documentation, tests, and earlier claims.

<a id="documentation-status"></a>

## Documentation status

The syntax, FFI, allocator, VM, JIT, memory model, and semantics
described here are works in progress. These pages explain current
behavior so it can be tested and challenged; they do not guarantee that
the described design, spelling, layout, ownership rules, or behavior
will remain unchanged. The v1 ABI is the exception: it is frozen for v1,
while v2 is expected to replace it.

TinyLang will not store every old version of the code, keep all
historical TinyLang versions available forever, or promise backward
compatibility any time soon. Users should pin the specific source
revision or release they need.

<a id="repository-and-documentation-drift"></a>

## Repository and documentation drift

- The old README referred to `Rust/` and root `stdlib/` paths. The live
  Rust crate is currently `crates/tinyone_core/` (crate name `tinylang`).
- Some historical planning documents still use `Rust/Cargo.toml` command
  examples; active user-facing docs use
  `crates/tinyone_core/Cargo.toml`.
- Historical release-helper examples may still assume `Rust/target` or
  `Rust/Cargo.toml`. Active tooling should use `crates/tinyone_core/`
  and excludes current `crates/tinyone_core/target` and
  `crates/tinyone_ralloc/target` build outputs by default.

<a id="test-and-verification-gaps"></a>

## Test and verification gaps

- The C FFI smoke test depends on a built debug `cdylib` and may skip
  when that library is not present.

<a id="language-and-runtime-gaps"></a>

## Language and runtime gaps

- The runtime type registry contains internal/staged variants beyond the
  source-level types. Heap `type_of` mappings are wired for all current
  `HeapData` variants; `TypeKind::try_from_runtime_value` returns `None`
  for a bare `HeapRef` because resolving its type requires heap context.
  The original `from_runtime_value` API remains available for
  compatibility and panics if passed a heap reference.
- The static/hybrid type-system direction is documented, but a full
  static type checker is not implemented yet.
- The peephole optimizer is conservative. It folds constant arithmetic
  and comparisons within basic blocks, remapping branch targets without
  moving expressions across control-flow boundaries.
- The adaptive JIT is not native code generation.
- Spawned functions use the portable VM backend even when the parent
  program uses JIT mode. Both parent modes provide the verified program
  reference and are covered by threading parity tests.

<a id="allocator-boundary"></a>

## Allocator boundary

- `crates/tinyone_ralloc/` (crate: `tinyone_ralloc`) is a path dependency
  of `tinyone_core` and is the backend for heap payloads, VM
  locals/globals, and native allocator sidecar entries.
- Transient VM/JIT operand stacks and compiler/JIT metadata still use
  Rust collections. They are control-plane data, not addressable TinyOne
  memory; moving them to Ralloc requires a separate variable-width value
  representation.
- The Ralloc arena remains capacity-bounded. VM, JIT, and spawned-thread
  frame allocation use the fallible `TinyMemory::try_new` path so
  exhaustion is a runtime error. `TinyMemory::new` and the standard
  `Clone` implementation retain conventional infallible APIs that
  document their panic behavior; embedders can use `try_new` and
  `try_clone` instead.

<a id="tests-and-benchmarks"></a>

# Tests and Benchmarks

Useful commands:

    cargo check --manifest-path crates/tinyone_core/Cargo.toml
    cargo test --manifest-path crates/tinyone_core/Cargo.toml
    cargo test --manifest-path crates/tinyone_core/Cargo.toml --features testing-hooks
    cargo build --release --manifest-path crates/tinyone_core/Cargo.toml --bin tinylang_bench
    ./target/release/tinylang_bench
    ./target/release/tinylang_bench --quick --repeats 1
    ./target/release/tinylang_bench --filter runtime.jit
    ./target/release/tinylang_bench --save-baseline tinyone-baseline.json
    ./target/release/tinylang_bench --save-baseline-auto
    ./target/release/tinylang_bench --baseline tinyone-baseline.json

The benchmark runner checks VM/JIT output parity before measuring and
reports best time, mean time, coefficient of variation, and per-thread
CPU work on both Windows and Linux. Windows uses scheduled thread cycles
and thread CPU time; Linux uses `CLOCK_THREAD_CPUTIME_ID` plus the TSC
cycle counter on x86/x86_64. Runtime rows reuse verified programs,
keeping compiler and verifier work out of dispatch timings. Its paired
rows are meant to make optimization work attributable: `allocator.*`
isolates Ralloc, `compiler.file_modules_*` compares full module
compilation with the size-aware disk-cache policy, `runtime.vm_*` and
`runtime.jit_*` compare execution tiers, and the paired
`runtime.jit_hot_loop_4096_*` rows isolate the adaptive JIT's
quickening benefit over a controlled 4,096-iteration loop. Collection
and heap phase rows cover 16, 256, and 4,096-entry workloads, individual
map and vector operations, generational slot reuse, and explicit frees.
Automatic baselines are written under `target/perf/<platform>/`
with machine, toolchain, Git, filesystem, and benchmark-option metadata.
Run the suite from an optimized build; debug-profile timing is not
representative.

File-backed benchmark fixtures default to the operating-system temporary
directory. Set `TINYONE_BENCH_FIXTURE_ROOT` to compare a particular
filesystem, such as WSL `/mnt/c` versus native `/tmp`.

When Windows and WSL share this checkout through `/mnt/c`, give Linux
its own Cargo artifact directory before building or testing:

    export CARGO_TARGET_DIR="$PWD/target/linux"

The Linux benchmark binary is then
`$CARGO_TARGET_DIR/release/tinylang_bench`. This keeps Linux and Windows
binaries and Cargo fingerprints from overwriting each other while
preserving the shared source worktree.

The built-in cycle column does not count retired native instructions and
Linux TSC cycles include descheduling time. On Linux, collect kernel
hardware counters around a filtered row when instruction counts or
branch behavior matter:

    perf stat -e cycles,instructions,branches,branch-misses -- \
      ./target/release/tinylang_bench \
      --filter runtime.jit_hot_loop_4096 --skip-correctness

See `docs/performance.md` for the workload map, measurement rules, and
the current optimization priority order.

Current state:

- `cargo check --manifest-path crates/tinyone_core/Cargo.toml` passes without
  warnings.
- The default test suite and feature-gated language fixture suite are
  release gates and are expected to pass before changes are pushed.

<a id="repository-layout"></a>

# Repository Layout

    .
    |-- build/                        cargo-make task files (ci, ffi, dist, docker, etc.)
    |-- Cargo.toml                    workspace root
    |-- crates/
    |   |-- tinyone_core/              compiler, VM, JIT, FFI, CLI (crate: tinylang)
    |   |   |-- src/
    |   |   |   |-- bin/              tinylang_bench.rs, tinyone-sandbox-worker.rs
    |   |   |   |-- bytecode/         opcode, instr, program, artifact, peephole, verifier
    |   |   |   |-- compiler/         parser, state, symbols, modules, incremental
    |   |   |   |-- jit/              op, chunk, program, cache, vm
    |   |   |   |-- runtime/          vm, heap, memory, value, stdlib, sync, pointers, ...
    |   |   |   |-- syntax/           lexer, token
    |   |   |   |-- api.rs, cli.rs, ffi.rs, runner.rs, builtins.rs, ...
    |   |   |   `-- tests/            abi_api_soundness, runtime_parity, stdlib_parity,
    |   |   |                         language_suite, language/, programs/, threading, ...
    |   |-- tinyone_ralloc/           Ralloc allocator crate (crate: tinyone_ralloc)
    |   |   |-- src/ + include/ralloc.h + tests/
    |   |-- tinyone_test_support/     shared test-support crate
    |   `-- xtask/                    release-gate / CI orchestrator
    |-- docs/                         abi/, ffi/, syntax/, architecture, bytecode, ...
    |-- scripts/                      check_abi_drift, consumer_compile/smoke
    |-- tests/consumers/              C / C++ / Rust FFI consumer fixtures
    |-- tools/                        hash.py, loc.py, abi_manifest.py, zip.py
    |-- tinylang.h                    generated C header (cbindgen)
    |-- tinyone-response-schema.json  committed JSON schema for ABI responses
    |-- Makefile.toml, build/         cargo-make task definitions
    |-- cbindgen.toml, cliff.toml     tool config
    |-- rust-toolchain.toml           toolchain pin
    `-- Config.toml                   TinyOne project policy (compile-cache deps)

<a id="release-direction"></a>

# Release Direction

Active v2 language development is documented in `docs/v2-roadmap.md`.
The v1 release themes include:

- stable v1 JSON response schemas
- frozen v1 C ABI policy
- safer verified-program execution typing
- clearer public/private bytecode program ownership
- better test coverage for Phase 2 builtins and artifact limits
- static/hybrid type-system work
- explicit numeric semantics
- deterministic ownership and allocator integration
- documentation cleanup after the crate path move

<a id="license"></a>

# License

See `LICENSE`.

<a id="feedback-and-community"></a>

# Feedback and Community

Comments, concerns, and questions should be sent to the TinyLang
community forum at <https://tl.404connernotfound.dev>. This is the
project channel for discussing the evolving design and reporting issues
with the current documentation or implementation.
