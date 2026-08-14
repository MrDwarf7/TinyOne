TinyLang
========

TinyOne is the v1 release generation of TinyLang, a portable systems
programming language designed around a compact VM, bounded runtime
assumptions, a small reasoning surface, and the freedom to discover its own
best design.

TinyLang is not tiny because its syntax is minimal. It is tiny where systems
usually become expensive: runtime footprint, allocator design, host
assumptions, VM behavior, platform dependencies, and implementation burden.

The current Rust implementation includes a lexer, compiler, bytecode
optimizer, verifier, portable VM, heap/runtime model, bytecode artifact
support, adaptive execution support, host integration surfaces, CLI tooling,
and early allocator-integration scaffolding.

Current crate version: ``1.2.0`` (the implementation is now managed as the
public v1 release line while language work proceeds internally under v2).

The current Rust crate lives in ``TinyOne/`` in this checkout. TinyLang is the
durable language identity and TinyOne is its v1 implementation generation.
TinyLang v2 is the active internal language roadmap: v2 features are developed
on the v1 release foundation without creating a separate user-facing language.

.. note::

   **TinyLang v2 Development Update**
   
   With the successful v1 release line locked in, active development has shifted
   to **TinyLang v2**. Key architectural upgrades currently underway include:
   
   * **Deterministic Binary FFI:** Replacing JSON-over-C with a zero-copy, 
     cross-system binary serialization protocol.
   * **Universal Language SDKs:** First-class test crates coming for **C, Python, 
     JavaScript (Node.js), and Rust**.
   * **Unified Memory Determinism:** Deepening ``Ralloc`` integration across all VM 
     and runtime primitives.
   
   *The frozen v1 ABI remains stable in ``TinyOne/`` while v2 features evolve internally.*

---

.. contents::

General Information
-------------------

- Source crate: ``TinyOne/``
- C FFI header: ``tinylang.h``
- Public documentation: ``docs/``
- Design notes and future-direction documents: ``Developer/``
- Developer tools: ``Tools/``
- Allocator work-in-progress: ``Ralloc/``

TinyLang is now in its v1 release line, but v1 and earlier are deliberately
discovery versions. The syntax, bytecode format, builtin set, JSON artifacts,
FFI details, allocator, VM, JIT, memory design, and semantics are subject to
change. The v1 ABI itself is frozen for the entire v1 lifecycle. Nothing in
the syntax, FFI, or allocator documentation is a promise of permanence or
perfection. These versions exist to test theories, kill stereotypes, and
proactively develop the model we are working toward.

TinyLang v2 will be a major version jump with massive changes, including a new
language-boundary design. The ABI is being saved for that major jump because
it defines how two languages communicate. Documentation describes current
behavior and design direction, not an unchangeable promise across v2.

TinyLang is working toward a language that leaves no excess overhead, uses the
smallest footprint it can without giving up reliability, and proactively pushes
the idea of necessary performance loss out the window. Reaching that goal means
changing the v2 ABI, FFI, allocator, VM, JIT, and surrounding implementation
model when the evidence says we should.

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

    cargo build --manifest-path TinyOne/Cargo.toml

Run the repo-local CI/release gate from the repository root::

    cargo run --manifest-path xtask/Cargo.toml -- release-gate

The gate checks TinyOne, Ralloc, the developer harness, language fixtures,
formatting, Clippy, benchmark smoke coverage, and Python tooling. It requires
``uv`` for the Python steps. ``scripts/ci-gate.sh`` remains available as a
Unix-oriented shell wrapper.

This creates the debug executable at ``TinyOne/target/debug/tinylang``. Build
with ``--release`` when you want the optimized executable at
``TinyOne/target/release/tinylang``. The Windows executable name is
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
* `
ull``
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
VM-owned heap with generation validation and explicit unsafe deallocation. Heap
payloads, VM locals/globals, and allocator-side backing are Ralloc-owned;
allocation-table bookkeeping and shutdown remain deterministic.

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

Returned strings must be released with ``tinyone_free_string``. The v1 ABI is
frozen for the entire v1 lifecycle. Check ``tinyone_abi_version()`` against
``TINYONE_ABI_VERSION`` before using the API; expect major incompatibilities
when TinyLang v2 introduces its new language-boundary design.

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
* ``docs/v2-roadmap.md`` tracks the active internal v2 language roadmap

The change-document process is defined by the TinyLang documentation-change
system:

``TLR``
    TinyLang Request. A lightweight request for a change, fix, clarification,
    or improvement.

``TLP``
    TinyLang Proposal. A structured design proposal for significant language,
    compiler, tooling, documentation, standard-library, ecosystem, or governance
    changes.

``TLIN``
    TinyLang Implementation Notice. A release-facing or pre-release notice
    explaining what is being implemented, what changed, and how users should
    migrate.

The intended path is::

    TLR -> TLP -> TLIN

Small accepted changes may go directly from TLR to TLIN. Major language changes
should not skip the TLP stage.

Developer Tools
---------------

``Tools/hash.py``
    Stdlib-only file, tree, and manifest hashing tool for release manifests,
    audit checkpoints, and source-tree integrity checks.

``Tools/loc.py``
    Small line-count and audit utility for source and documentation files.

Examples::

    python3 Tools/hash.py README.rst
    python3 Tools/hash.py --tree . --format json --list-files
    python3 Tools/hash.py --check manifest.json
    python3 Tools/loc.py --audit --docs --json

Known Implementation Gaps
-------------------------

This section intentionally records gaps between current implementation,
documentation, tests, and earlier claims.

Documentation status
^^^^^^^^^^^^^^^^^^^^

The syntax, FFI, allocator, VM, JIT, memory model, and semantics described here
are works in progress. These pages explain current behavior so it can be tested
and challenged; they do not guarantee that the described design, spelling,
layout, ownership rules, or behavior will remain unchanged. The v1 ABI is the
exception: it is frozen for v1, while v2 is expected to replace it.

TinyLang will not store every old version of the code, keep all historical
TinyLang versions available forever, or promise backward compatibility any time
soon. Users should pin the specific source revision or release they need.

Repository and documentation drift
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

* The old README referred to ``Rust/`` and root ``stdlib/`` paths. The live Rust
  crate is currently under ``TinyOne/``.
* Some historical planning documents still use ``Rust/Cargo.toml`` command
  examples; active user-facing docs use ``TinyOne/Cargo.toml``.
* Historical release-helper examples may still assume ``Rust/target`` or
  ``Rust/Cargo.toml``. Active tooling should use ``TinyOne/`` and excludes
  current ``TinyOne/target`` and ``Ralloc/target`` build outputs by default.

Test and verification gaps
^^^^^^^^^^^^^^^^^^^^^^^^^^

* The C FFI smoke test depends on a built debug ``cdylib`` and may skip when
  that library is not present.

Language and runtime gaps
^^^^^^^^^^^^^^^^^^^^^^^^^

* The runtime type registry contains internal/staged variants beyond the
  source-level types. Heap ``type_of`` mappings are wired for all current
  ``HeapData`` variants; ``TypeKind::try_from_runtime_value`` returns ``None``
  for a bare ``HeapRef`` because resolving its type requires heap context. The
  original ``from_runtime_value`` API remains available for compatibility and
  panics if passed a heap reference.
* The static/hybrid type-system direction is documented, but a full static type
  checker is not implemented yet.
* The peephole optimizer is conservative. It folds branch-free constant
  arithmetic/comparison chunks and intentionally avoids chunks with jumps.
* The adaptive JIT is not native code generation.
* Spawned functions use the portable VM backend even when the parent program
  uses JIT mode. Both parent modes provide the verified program reference and
  are covered by threading parity tests.

Allocator boundary
^^^^^^^^^^^^^^^^^^

* ``Ralloc/`` is a path dependency of ``TinyOne`` and is the backend for heap
  payloads, VM locals/globals, and native allocator sidecar entries.
* Transient VM/JIT operand stacks and compiler/JIT metadata still use Rust
  collections. They are control-plane data, not addressable TinyOne memory;
  moving them to Ralloc requires a separate variable-width value
  representation.
* The Ralloc arena remains capacity-bounded. VM, JIT, and spawned-thread frame
  allocation use the fallible ``TinyMemory::try_new`` path so exhaustion is a
  runtime error. ``TinyMemory::new`` and the standard ``Clone`` implementation
  retain conventional infallible APIs that document their panic behavior;
  embedders can use ``try_new`` and ``try_clone`` instead.

Tests and Benchmarks
--------------------

Useful commands::

    cargo check --manifest-path TinyOne/Cargo.toml
    cargo test --manifest-path TinyOne/Cargo.toml
    cargo test --manifest-path TinyOne/Cargo.toml --features testing-hooks
    cargo build --release --manifest-path TinyOne/Cargo.toml --bin tinylang-bench
    ./TinyOne/target/release/tinylang-bench
    ./TinyOne/target/release/tinylang-bench --quick --repeats 1

Current state:

* ``cargo check --manifest-path TinyOne/Cargo.toml`` passes without warnings.
* The default test suite and feature-gated language fixture suite are release
  gates and are expected to pass before changes are pushed.

Repository Layout
-----------------

::

    .
    |-- README.rst
    |-- License.rst
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
    |   `-- v2-roadmap.md
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
The user-facing language name is TinyLang. TinyOne names the v1 release
generation of the implementation line; v2 is the active internal roadmap.

Release Direction
-----------------

Active v2 language development is documented in ``docs/v2-roadmap.md``. The v1 release
themes include:

* stable v1 JSON response schemas
* frozen v1 C ABI policy
* safer verified-program execution typing
* clearer public/private bytecode program ownership
* better test coverage for Phase 2 builtins and artifact limits
* static/hybrid type-system work
* explicit numeric semantics
* deterministic ownership and allocator integration
* documentation cleanup after the crate path move

License
-------

See ``License.rst``.

Feedback and Community
----------------------

Comments, concerns, and questions should be sent to the TinyLang community
forum at https://tl.404connernotfound.dev. This is the project channel for
discussing the evolving design and reporting issues with the current
documentation or implementation.
