Project is on a temp hiatus
============================

TinyLang
=======

TinyOne is the first major generation of TinyLang, a portable systems
programming language designed around a compact VM, bounded runtime
assumptions, stable host-integration direction, and a small reasoning surface.

TinyLang is not tiny because its syntax is minimal. It is tiny where systems
usually become expensive: runtime footprint, allocator design, host
assumptions, VM behavior, platform dependencies, and implementation burden.

The current Rust implementation includes a lexer, compiler, bytecode
optimizer, verifier, portable VM, heap/runtime model, bytecode artifact
support, adaptive execution support, host integration surfaces, CLI tooling,
and early allocator-integration scaffolding.

Current crate version: ``0.6.0``.

The current Rust crate lives in ``TinyOne/`` in this checkout. TinyLang is the
durable language identity. TinyOne is the current beta implementation line for
the intended v1 generation, not a separate language that users must relearn.
Future major generations may use names such as TinyTwo or TinyThree while
remaining generations of TinyLang.

.. contents::

General Information
-------------------

- Source crate: ``TinyOne/``
- C FFI header: ``tinylang.h``
- Public documentation: ``docs/``
- Design notes and future-direction documents: ``Developer/``
- Developer tools: ``Tools/``
- Allocator work-in-progress: ``Ralloc/``

TinyLang is pre-1.0 software. The language, bytecode format, builtin set,
JSON artifacts, C ABI, and documentation process are still allowed to change
while the implementation is being brought into line with the intended v1
surface.

What Tiny Means
---------------

TinyLang is designed to stay small at the architectural level, not necessarily
at the syntax level.

The language may expose a broad syntax, many types, builtins, default syntax,
VM/runtime support, host interop, and a growing standard library. The
constraint is that each feature should preserve a compact operational model.
When code, syscalls, assembly, platform assumptions, or runtime machinery can
be removed without damaging reasoning or capability, they should be removed.

The largest implementation pieces may be the VM and memory allocator, but even
those should remain compact, inspectable, and understandable. The goal is a
capable language with a compact operational core that remains portable across
systems.

Project Goals
-------------

TinyLang is intended to be a capable all-in-one language/runtime implementation
with a compact operational core for:

* low-level programming
* high-level integration
* explicit memory and pointer work
* VM-enforced runtime safety
* deterministic non-GC cleanup direction
* practical multithreaded workloads
* compiler, bytecode, verifier, VM, JIT, FFI, and allocator education

TinyLang does not aim to hide unsafe operations. Operations that can affect
memory, runtime state, pointer provenance, or host resources should be explicit
and checked by the runtime wherever possible.

Build Instructions
------------------

Build the Rust crate and CLI executable::

    cargo build --manifest-path crates/tinyone_core/Cargo.toml

Run the repo-local CI/release gate from the repository root::

    scripts/ci_gate.sh

The gate runs the practical current checks and reports removed-stdlib fallout
honestly instead of restoring or assuming a root ``stdlib/`` tree.

This creates the debug executable at ``crates/tinyone_core/target/debug/tinylang``. Build
with ``--release`` when you want the optimized executable at
``crates/tinyone_core/target/release/tinylang``. The Windows executable name is
``tinylang.exe``. The examples below assume the executable is available on
``PATH`` as ``tinylang``.

Run the command-line tool directly::

    tinylang --help

Run a source file with the default adaptive JIT mode::

    tinylang program.to

Run the same source through the portable VM::

    tinylang --mode vm program.to

Compile and verify without running::

    tinylang --check program.to

Emit bytecode and JIT listings::

    tinylang --emit-bytecode program.tobc.json program.to
    tinylang --emit-jit program.jit.txt program.to

Run a bytecode artifact::

    tinylang --run-bytecode program.tobc.json

Command Line
------------

The CLI supports::

    usage: tinylang [OPTIONS] [path]

    Options:
      --mode {jit,vm}       Execution mode (default: jit)
      --check               Compile only, do not run
      --emit-bytecode PATH  Write a bytecode artifact to PATH
      --emit-jit PATH       Write a JIT listing to PATH
      --run-bytecode PATH   Run a compiled bytecode artifact
      --input VALUE         Supply a program input value (repeatable)
      --stdin               Read input values from stdin
      --verbose             Print program metadata before running
      -h, --help            Show help

Language Overview
-----------------

The implemented language currently includes:

* integer and string literals
* ``null``
* ``let`` bindings
* assignment to existing variables
* expression statements
* ``print``
* ``if``, ``else if``, and ``else``
* ``while``
* ``break`` and ``continue``
* top-level ``fn`` declarations
* top-level ``struct`` declarations
* arrays
* strings as heap objects
* structs as heap objects
* pointer cells through ``alloc``, ``load``, ``store``, and ``unsafe free``
* raw pointers for objects, arrays, struct fields, buffers, and cells
* unsafe-gated pointer arithmetic, raw loads/stores, and buffer reads/writes
* imports with namespaces and ``tinyone.json`` manifest resolution
* exported module declarations
* deterministic input through ``--input`` and ``--stdin``
* fixed-width runtime integer values for low-level memory work
* boolean operators ``&&``, ``||``, and ``!`` producing ``0`` or ``1``

Example::

    fn add(left, right) {
      return left + right
    }

    let answer = add(40, 2)
    print answer

Current compiler constraints:

* functions and structs are top-level only
* functions must be defined before ordinary calls
* recursive self-calls are supported from inside the function body
* nested functions are rejected
* top-level executable statements are rejected inside imported modules
* imports must appear before declarations or executable statements
* functions may read earlier top-level variables, but direct assignment to
  top-level slots from inside functions is rejected
* ``compile_file`` supports import resolution; ``compile_source`` style APIs do
  not resolve imports because they compile anonymous source without a resolver

Runtime and Memory Model
------------------------

TinyLang runs through this pipeline::

    source -> lexer -> compiler -> bytecode -> peephole optimizer -> verifier -> VM/JIT

The runtime includes:

* fixed-slot stack frames
* a generational heap slab
* heap references with generation checks
* raw pointer values with runtime provenance checks
* explicit manual deallocation through unsafe operations
* checked arithmetic and checked division
* resource limits for arrays, buffers, heap payload, heap object slots, nested
  calls, artifacts, verifier work, and filesystem reads
* shutdown heap draining through report APIs

TinyLang does not use a tracing garbage collector. The current runtime uses a
VM-owned heap with generation validation and explicit unsafe deallocation. The
longer-term design documents describe deterministic cleanup, indexed reference
counting, global allocation-table authority, region-based lifetime batching,
and Ralloc-backed allocation work.

VM and JIT
----------

TinyLang has two execution backends:

``vm``
    Portable bytecode interpreter. It is the simpler backend and is the main
    reference path for behavior checks.

``jit``
    Adaptive lowered-bytecode tier. It is not a native machine-code JIT. It
    compiles verified bytecode into internal JIT ops, caches compiled programs
    by fingerprint, emits inspectable listings, and quickens hot back edges.

Both public run paths verify bytecode before execution.

C FFI
-----

The crate builds as an ``rlib`` and ``cdylib``. The C header is ``tinylang.h``.

The FFI surface uses JSON-over-C-string entry points:

* ``tinyone_lex_source_json``
* ``tinyone_compile_source_json``
* ``tinyone_compile_file_json``
* ``tinyone_run_source_json``
* ``tinyone_run_file_json``
* ``tinyone_run_artifact_json``
* ``tinyone_jit_listing_json``
* ``tinyone_free_string``

Returned strings must be released with ``tinyone_free_string``. The ABI is
explicitly unstable before v1.

Documentation
-------------

The main documentation tree is ``docs/``:

* ``docs/index.md`` routes readers by audience
* ``docs/syntax/`` describes syntax
* ``docs/abi/`` describes ABI contracts and versioning
* ``docs/ffi/`` describes C and Rust integration
* ``docs/architecture.md`` describes the pipeline and module map
* ``docs/bytecode.md`` describes opcodes, artifacts, verifier rules, and JIT
* ``docs/memory-model.md`` describes heap handles, pointer checks, and limits
* ``docs/stdlib.md`` describes builtins and stdlib bridge behavior
* ``docs/v1-roadmap.md`` tracks stable-ABI blockers

The change-document process is defined by the TinyLang documentation-change
system:

``TOR``
    TinyLang Request. A lightweight request for a change, fix, clarification,
    or improvement.

``TOP``
    TinyLang Proposal. A structured design proposal for significant language,
    compiler, tooling, documentation, standard-library, ecosystem, or governance
    changes.

``TOIN``
    TinyLang Implementation Notice. A release-facing or pre-release notice
    explaining what is being implemented, what changed, and how users should
    migrate.

The intended path is::

    TOR -> TOP -> TOIN

Small accepted changes may go directly from TOR to TOIN. Major language changes
should not skip the TOP stage.

Developer Tools
---------------

``tools/hash.py``
    Stdlib-only file, tree, and manifest hashing tool for release manifests,
    audit checkpoints, and source-tree integrity checks.

``tools/loc.py``
    Small line-count and audit utility for source and documentation files.

Examples::

    python3 tools/hash.py README.rst
    python3 tools/hash.py --tree . --format json --list-files
    python3 tools/hash.py --check manifest.json
    python3 tools/loc.py --audit --docs --json

Known Implementation Gaps
-------------------------

This section intentionally records gaps between current implementation,
documentation, tests, and earlier claims.

Repository and documentation drift
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

* The old README referred to ``Rust/`` and root ``stdlib/`` paths. The live Rust
  crate is currently under ``TinyOne/``.
* Some historical planning documents still use ``Rust/Cargo.toml`` command
  examples; active user-facing docs use ``crates/tinyone_core/Cargo.toml``.
* Some tests still assume a root ``stdlib/tinyone.json`` manifest. That root
  ``stdlib/`` tree is intentionally absent in the current checkout while the
  standard-library surface moves into the runtime/system layer.
* Historical release-helper examples may still assume ``Rust/target`` or
  ``Rust/Cargo.toml``. Active tooling should use ``TinyOne/`` and excludes
  current ``TinyOne/target`` and ``Ralloc/target`` build outputs by default.
* Some docs describe raw pointer kinds such as ``struct`` and ``cell`` while
  the implementation uses object, array, buffer, field, and null pointer kinds.
* Some docs describe generation changes as happening on free; the current heap
  increments generations when a slot is reused for allocation.

Test and verification gaps
^^^^^^^^^^^^^^^^^^^^^^^^^^

* ``cargo test --manifest-path crates/tinyone_core/Cargo.toml`` currently fails in
  ``stdlib_modules_compile_via_manifest_import`` because ``stdlib/tinyone.json``
  is missing.
* ``cargo test --manifest-path crates/tinyone_core/Cargo.toml --features testing-hooks``
  currently has testing-hook type drift: the test facade derives ``Eq`` for a
  structure containing ``Vec<RuntimeValue>`` while ``RuntimeValue`` is only
  ``PartialEq``, and the facade still has ``Program``/``Arc<Program>`` mismatch
  points.
* The default crate build currently emits warnings for unused imports,
  variables, fields, methods, and staged heap variants.
* The C FFI smoke test depends on a built debug ``cdylib`` and may skip when
  that library is not present.

Language and runtime gaps
^^^^^^^^^^^^^^^^^^^^^^^^^

* Enum syntax appears in fixtures, but the live lexer/parser do not implement
  ``enum`` syntax yet.
* Type annotations, float literals, and boolean literal syntax appear in newer
  fixture names, but the current lexer/parser do not implement ``:``, ``->``,
  floats, or ``true``/``false`` language syntax as first-class tokens.
* Some 43-type runtime variants are representable but not fully behavior-wired.
  Several newer heap/type paths still use explicit ``unimplemented!`` stubs.
* The static/hybrid type-system direction is documented, but a full static type
  checker is not implemented yet.
* The peephole optimizer is conservative. It folds branch-free constant
  arithmetic/comparison chunks and intentionally avoids chunks with jumps.
* The adaptive JIT is not native code generation.
* ``thread_spawn`` support is VM-oriented today. The JIT context path does not
  appear to set the program reference needed by ``thread_spawn``.
* Mutex unlock currently checks locked/unlocked state but does not prove the
  unlocking thread is the owner.

Allocator gaps
^^^^^^^^^^^^^^

* ``TinyAllocator`` is currently a tracking and hook scaffold. It does not yet
  back TinyLang heap allocations with real Ralloc native allocations.
* The in-repository ``Ralloc/`` tree is an embedded allocator workspace, but it
  is not wired as a dependency of the TinyLang crate.
* The Phase 2 allocator documents call out required future work: real Ralloc
  backing, allocator sidecar storage, atomic table update during reallocate,
  larger/dynamic arena capacity, real thread ownership, and global allocation
  table state expansion.

Tests and Benchmarks
--------------------

Useful commands::

    cargo check --manifest-path crates/tinyone_core/Cargo.toml
    cargo test --manifest-path crates/tinyone_core/Cargo.toml
    cargo test --manifest-path crates/tinyone_core/Cargo.toml --features testing-hooks
    cargo build --release --manifest-path crates/tinyone_core/Cargo.toml --bin tinylang_bench
    ./crates/tinyone_core/target/release/tinylang_bench
    cargo build --release --manifest-path crates/tinyone_core/Cargo.toml --bin tinylang_bench
    ./crates/tinyone_core/target/release/tinylang_bench --quick --repeats 1

Current state:

* ``cargo check --manifest-path crates/tinyone_core/Cargo.toml`` passes, but with warnings.
* The default test suite is not clean because of the missing root stdlib
  manifest.
* The feature-gated language fixture suite needs testing-hook repairs before it
  can be treated as a clean verification gate.

Repository Layout
-----------------

::

    .
    |-- README.rst
    |-- LICENSE.md
    |-- tinylang.h
    |-- TinyOne/
    |   |-- Cargo.toml
    |   |-- src/
    |   |-- tests/
    |   `-- Cargo.lock
    |-- docs/
    |   |-- abi/
    |   |-- ffi/
    |   |-- syntax/
    |   |-- architecture.md
    |   |-- bytecode.md
    |   |-- memory-model.md
    |   |-- stdlib.md
    |   `-- v1-roadmap.md
    |-- Developer/
    |   |-- typing_system.md
    |   |-- ownership_semantics_and_memory_safety.md
    |   |-- phase_2.md
    |   `-- phase_2_allocator.md
    |-- Tools/
    |   |-- hash.py
    |   `-- loc.py
    `-- Ralloc/
        |-- Cargo.toml
        |-- src/
        |-- include/
        `-- tests/

The ``TinyOne/`` directory name is still present on disk for the Rust crate.
The user-facing language name is TinyLang. TinyOne names this major generation
of the TinyLang implementation line.

Release Direction
-----------------

The v1 direction is documented in ``docs/v1-roadmap.md`` and the design notes
under ``Developer/``. Major themes include:

* stable JSON response schemas
* stable C ABI policy
* safer verified-program execution typing
* clearer public/private bytecode program ownership
* better test coverage for Phase 2 builtins and artifact limits
* static/hybrid type-system work
* explicit numeric semantics
* deterministic ownership and allocator integration
* documentation cleanup after the crate path move

License
-------

See ``LICENSE.md``.
