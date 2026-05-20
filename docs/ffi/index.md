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
