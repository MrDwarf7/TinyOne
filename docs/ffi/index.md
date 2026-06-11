# FFI Integration

TinyOne compiles to a shared library (`libtinyone.so` / `.dylib` / `.dll`)
and a Rust crate. This section covers how to integrate TinyOne into a host
application from both C and Rust.

For the ABI contract and JSON schema reference, see [`abi/`](../abi/index.md).

## Documents in This Area

| File | Description |
| --- | --- |
| [`c-integration.md`](c-integration.md) | Building, linking, and calling TinyOne from C or C++; entry-point reference; ownership and threading rules; complete code examples |
| [`rust-api.md`](rust-api.md) | Rust crate public API: compilation, execution, JIT, artifact I/O, and verification functions |

## ABI Drift Tooling

The generated C compatibility header is `tinylang.h`. It is generated from the
Rust FFI source when `cbindgen` is available, while exported C symbols keep the
existing `tinyone_*` names.

Use the no-dependency drift check before changing `TinyOne/src/ffi.rs` or
`tinylang.h`:

```sh
./scripts/check-abi-drift.sh
```

For review artifacts, emit a deterministic symbol manifest:

```sh
python3 Tools/abi_manifest.py manifest
```

If `cbindgen` is installed, the same tool can attempt the planned shim header:

```sh
python3 Tools/abi_manifest.py generate-header --output tinylang.h
```

When `cbindgen` is not on `PATH`, generation fails with a clear message and the
`check`/`manifest` commands continue to work without network installs.
