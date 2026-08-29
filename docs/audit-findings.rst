V1 Audit Findings
=================

This audit covers the TinyOne v1 implementation under ``TinyOne/``.
References use repository-relative paths so they remain valid as line numbers
change.

Confirmed boundaries
--------------------

* **FFI panic containment:** All exported JSON entry points in
  ``TinyOne/src/ffi.rs`` funnel through the guarded response path. The
  committed C header defines string ownership, nullability, input limits, and
  the ABI version check.
* **Verified execution:** Public VM/JIT dispatch verifies bytecode before
  using unchecked internal constructors. Artifact parsing applies collection
  and verifier budgets before execution.
* **Recoverable frame allocation:** VM, JIT, recursive-function, and
  spawned-thread frames use ``TinyMemory::try_new``, preserving Ralloc
  exhaustion and size overflow as ``TinyOneError`` values. The explicitly
  infallible ``TinyMemory::new`` and ``Clone`` APIs document their panic
  behavior; embedders can use ``try_new`` and ``try_clone``.
* **Mutex ownership:** ``TinyMutex::unlock`` rejects unlocked mutexes and
  calls from threads other than the recorded owner.
* **Heap generations:** Freeing makes a slot vacant; its generation increments
  immediately before the slot is reused. Heap references and raw pointers are
  validated against the current generation.
* **Closed ABI response shapes:** ABI version 1 success and error objects
  reject unknown keys. The committed schema and exact-key contract tests cover
  every exported JSON endpoint.

Intentional constraints
-----------------------

* Spawned function bodies execute on the portable VM backend even when their
  parent is running in JIT mode. Threading integration tests exercise both
  parent modes.
* The C smoke test requires a built debug dynamic library and may skip when
  the library is unavailable. Rust-level ABI contract tests remain
  unconditional.
* A bare ``HeapRef`` does not contain its heap object's type. Consequently,
  ``TypeKind::try_from_runtime_value`` returns ``None`` for heap references;
  callers must resolve those through the owning heap. The original
  ``TypeKind::from_runtime_value`` signature is retained for compatibility and
  panics when given a heap reference.
