# ABI Reference

TinyOne's C ABI is the interface between the compiled `libtinyone` shared
library and any host application. It covers the JSON response contract,
the panic boundary, ownership rules, and the versioning policy.

**ABI status: UNSTABLE** until v1 is tagged. See
[versioning.md](versioning.md) for what that means in practice.

For how to link and call the library from C, see
[`ffi/c-integration.md`](../ffi/c-integration.md).

## Documents in This Area

| File | Description |
| --- | --- |
| [`contract.md`](contract.md) | Runtime invariants callers can rely on today: panic boundary, null safety, ownership, thread safety, verification guarantee |
| [`versioning.md`](versioning.md) | What constitutes a breaking change, current stability status per area, and the v1 stability declaration plan |
| [`schemas.md`](schemas.md) | Exact JSON `value` schemas for every entry point's success response |
