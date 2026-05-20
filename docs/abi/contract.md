# TinyOne ABI Contract

These invariants hold for all current entry points. They are not subject
to the ABI stability question — they describe observable behavior that
TinyOne guarantees today and will preserve across versions.

See [`versioning.md`](versioning.md) for what changes before v1 and
what freezes after. See [`schemas.md`](schemas.md) for the exact JSON
field contracts per endpoint.

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

Every `const char *` parameter that is not marked `/* nullable */` in
`tinyone.h` accepts a null pointer without crashing. Passing null returns
a structured compile error:

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
