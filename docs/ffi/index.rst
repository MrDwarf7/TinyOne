FFI Integration
===============

The FFI documentation describes the current v1 integration surface. The v1
ABI is frozen for the v1 lifecycle, but FFI behavior and supporting
implementation details may change as TinyLang develops. TinyLang v2 is
expected to make major changes to how language boundaries work.

Send FFI comments, concerns, and questions to the `TinyLang community
forum <https://tl.404connernotfound.dev>`_.

TinyOne compiles to a shared library (``libtinyone.so``, ``.dylib``, or
``.dll``) and a Rust crate. This section covers how to integrate TinyOne into
a host application from both C and Rust.

For the ABI contract and JSON schema reference, see
`abi/index.rst <../abi/index.rst>`_.

Documents in This Area
----------------------

* `c-integration.md <c-integration.md>`_ -- Building, linking, and calling
  TinyOne from C or C++; entry-point reference; ownership and threading rules;
  complete code examples.
* `rust-api.md <rust-api.md>`_ -- Rust crate public API: compilation,
  execution, JIT, artifact I/O, and verification functions.

ABI Drift Tooling
-----------------

The generated C compatibility header is ``tinylang.h``. It is generated from
the Rust FFI source when ``cbindgen`` is available, while exported C symbols
keep the existing ``tinyone_*`` names.

Use the no-dependency drift check before changing ``TinyOne/src/ffi.rs`` or
``tinylang.h``::

   ./scripts/check-abi-drift.sh

For review artifacts, emit a deterministic symbol manifest::

   python3 Tools/abi_manifest.py manifest

If ``cbindgen`` is installed, the same tool can attempt the planned shim
header::

   python3 Tools/abi_manifest.py generate-header --output tinylang.h

When ``cbindgen`` is not on ``PATH``, generation fails with a clear message
and the ``check``/``manifest`` commands continue to work without network
installs.
