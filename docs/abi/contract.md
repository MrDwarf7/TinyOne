---
title: ABI Contract
---

# TinyOne ABI Contract

These invariants are the frozen ABI version 1 contract. They remain frozen for
the entire v1 lifecycle. TinyLang v2 is expected to make massive changes to
the language boundary, so this contract does not promise compatibility across
the v2 version jump.

TinyLang does not promise to retain every historical implementation or keep
old versions available forever. For comments, concerns, or questions, use the
[TinyLang community forum](https://tl.404connernotfound.dev).

See [`schemas.md`](schemas.md) for the exact JSON field contracts per endpoint.

## Panic Boundary

Every `char *`-returning entry point is wrapped in two nested
`catch_unwind` guards (`ffi.rs: respond()`). If Rust code panics for
any reason, the panic is caught and reported as:

```json
{"ok": false, "kind": "panic", "error": "TinyOne panicked across the FFI boundary"}
```

The caller's stack is never unwound. The `{"ok":false,"kind":"panic"}`
shape should never appear in normal use — it indicates a library bug.

## Null Safety

Every required `const char *` parameter is documented as non-null in
`tinylang.h`. Passing null still returns a structured compile error rather
than crashing:

```json
{"ok": false, "kind": "compile", "error": "... pointer was null"}
```

The `inputs_json` parameter in all `run_*` functions is explicitly
nullable. Passing `NULL` is equivalent to passing an empty input queue.

## Ownership

Every `char *` returned by a `tinyone_*_json` function is a
heap-allocated, NUL-terminated UTF-8 string. The caller is responsible
for freeing it with `tinyone_free_string`. Freeing with the C standard
`free()` is undefined behavior.

`tinyone_free_string(NULL)` is always safe and is a no-op.

A non-null argument must be an outstanding pointer returned by one of the JSON
entry points and must be freed exactly once.

Do not share a returned `char *` pointer across threads without
synchronization; free it from the same thread that called the function,
or transfer ownership with appropriate synchronization.

## Response Envelope

All entry points return one of four JSON shapes:

```json
{"ok": true, "value": { ... }}
{"ok": false, "kind": "compile",  "error": "message"}
{"ok": false, "kind": "runtime",  "error": "message"}
{"ok": false, "kind": "panic",    "error": "TinyOne panicked across the FFI boundary"}
```

`"ok"` is always present and always a boolean. When `"ok"` is `true`,
`"value"` is present. When `"ok"` is `false`, `"kind"` and `"error"`
are present. No other top-level keys appear in any response.

## Thread Safety

TinyOne has no global mutable state in its public API. Each FFI call is
fully self-contained. You may call entry points from multiple threads
simultaneously if each call uses a distinct set of arguments.

`JitCache` maintains mutable state and is not thread-safe. The FFI
`run_*` functions each create a fresh `JitCache` per call, so they are
independently thread-safe.

## Verification Before Execution

Every execution path (VM and JIT, FFI and Rust API) runs
`BytecodeVerifier::verify` exactly once before any instruction executes.
A program that passes verification will not crash the host process due
to malformed bytecode — all bytecode errors surface as structured
`TinyOneError` values.

## Text Input Limits

The C ABI bounds every NUL-terminated text parameter before parsing or
compilation. Source text is limited to 1 MiB, paths to 32 KiB, execution modes
to 16 bytes, and `inputs_json` to 8 MiB, excluding the trailing NUL. Source
files loaded by `compile_file` and imports use the same 1 MiB source limit.
Oversized values return a structured compile error and are not parsed.

## Process Sandbox

The `run_source`, `run_file`, `run_artifact`, and JIT-listing C entry points
execute inside the dedicated `tinyone-sandbox-worker` process. The parent waits
for at most five seconds, then terminates the worker and returns a structured
runtime error. The worker must be installed beside the host executable, or its
path may be supplied through `TINYONE_SANDBOX_WORKER`.

This is process isolation and deadline enforcement, not an OS security policy:
the worker inherits the host account's filesystem and other operating-system
permissions. Use a restricted account/container/job object when the input is
from an untrusted principal.
