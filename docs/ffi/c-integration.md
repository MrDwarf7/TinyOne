# TinyOne C FFI Integration Guide

TinyOne builds as a `cdylib` alongside the CLI binary. All public entry points
are declared in `tinylang.h` at the repository root. This document covers how to
embed TinyOne in a C or C++ application.

**ABI STATUS: UNSTABLE.** Do not pin to a specific ABI version until v1 is
tagged and the ABI is declared stable.

## Building the Library

```sh
# Debug (for development and testing)
cargo build --manifest-path TinyOne/Cargo.toml

# Release (for embedding)
cargo build --release --manifest-path TinyOne/Cargo.toml
```

Output locations:

| Platform | Debug | Release |
| --- | --- | --- |
| Linux | `TinyOne/target/debug/libtinyone.so` | `TinyOne/target/release/libtinyone.so` |
| macOS | `TinyOne/target/debug/libtinyone.dylib` | `TinyOne/target/release/libtinyone.dylib` |
| Windows | `TinyOne/target/debug/tinyone.dll` | `TinyOne/target/release/tinyone.dll` |

## Linking

```sh
# Linux
cc -std=c11 your_app.c -I/path/to/repo -L/path/to/TinyOne/target/release \
   -Wl,-rpath,/path/to/TinyOne/target/release -ltinyone -o your_app

# macOS
cc -std=c11 your_app.c -I/path/to/repo -L/path/to/TinyOne/target/release \
   -rpath @loader_path -ltinyone -o your_app
```

Include the header:

```c
#include "tinylang.h"
```

## Header Drift Checks

The committed generated C header for the current ABI is `tinylang.h`. It keeps
exported symbols named `tinyone_*`; do not rename the C symbols as part of
ordinary header work.

Before changing `TinyOne/src/ffi.rs` or `tinylang.h`, run:

```sh
./scripts/check-abi-drift.sh
```

That command uses only Python's standard library and compares exported
`extern "C"` Rust symbols against `tinylang.h`. A deterministic manifest is
available for review:

```sh
python3 Tools/abi_manifest.py manifest
```

Header generation is optional and requires a local `cbindgen` binary:

```sh
python3 Tools/abi_manifest.py generate-header --output tinylang.h
```

If `cbindgen` is missing, the tool reports that explicitly and still supports
the no-dependency drift check.

## Ownership Contract

Every function that returns `char *` returns a **heap-allocated, NUL-terminated
UTF-8 string**. The caller must free it with `tinyone_free_string`. Do not call
the C standard library `free()` — the allocator may differ.

```c
char *result = tinyone_run_source_json("print 42", "jit", NULL);
// ... use result ...
tinyone_free_string(result);  // required
```

`tinyone_free_string(NULL)` is always safe and is a no-op.

## JSON Response Format

All functions return a JSON string. There are four response shapes:

```json
{"ok": true,  "value": { ... }}
{"ok": false, "kind": "compile",  "error": "message"}
{"ok": false, "kind": "runtime",  "error": "message"}
{"ok": false, "kind": "panic",    "error": "TinyOne panicked across the FFI boundary"}
```

The `"panic"` shape should never occur in normal use — it indicates a library
bug. All internal panics are caught at the FFI boundary and reported as JSON
rather than unwinding into the caller.

## Parameter Nullability

Unless a parameter is annotated `/* nullable */` in `tinylang.h`, it must be a
valid NUL-terminated UTF-8 C string. Passing `NULL` for a non-nullable parameter
does **not** crash — it returns a structured `{"ok":false,"kind":"compile","error":"… pointer was null"}` response.

The `inputs_json` parameter in all `run_*` functions is nullable. Passing `NULL`
is equivalent to passing an empty input queue.

## Entry Points

### `tinyone_free_string`

```c
void tinyone_free_string(char *value);
```

Free a string returned by any `tinyone_*_json` function. Calling with `NULL` is
safe. Do not call on any pointer not returned by this library.

---

### `tinyone_lex_source_json`

```c
char *tinyone_lex_source_json(const char *source);
```

Lex `source` and return the number of tokens.

**Success:**
```json
{"ok": true, "value": {"tokens": 5}}
```

**Error:** compile error or null source pointer.

---

### `tinyone_compile_source_json`

```c
char *tinyone_compile_source_json(const char *source);
```

Compile `source` through the full pipeline (lex → compile → optimize → verify).
Returns a bytecode artifact and its fingerprint.

**Success:**
```json
{
  "ok": true,
  "value": {
    "artifact":    { "format": "tinyone-bytecode", "version": 1, … },
    "fingerprint": "a1b2c3d4e5f60718"
  }
}
```

**Error:** compile error or null pointer.

---

### `tinyone_compile_file_json`

```c
char *tinyone_compile_file_json(const char *path);
```

Same as `tinyone_compile_source_json` but reads source from `path` on disk.
The path is canonicalized before reading; relative paths are resolved from the
process working directory.

---

### `tinyone_run_source_json`

```c
char *tinyone_run_source_json(
    const char *source,
    const char *mode,
    const char *inputs_json  /* nullable */
);
```

Compile and run `source`. `mode` must be `"vm"` or `"jit"`. `inputs_json` is an
optional JSON array of strings that pre-populate the deterministic input queue
consumed by `read()`, `read_int()`, and `read_str()`.

**Success:**
```json
{
  "ok": true,
  "value": {
    "stdout": "42\n",
    "memory": [
      {"type": "int",     "value": 42},
      {"type": "heap",    "address": 0, "generation": 1},
      {"type": "pointer", "address": 0, "kind": "array", "index": 0,
                          "field": null, "generation": 1, "cast": null}
    ],
    "heap_before_shutdown": {
      "live_objects": 1, "live_bytes": 64,
      "peak_objects": 1, "peak_bytes": 64,
      "total_allocations": 1, "total_frees": 0, "shutdown_frees": 0
    },
    "heap_after_shutdown": {
      "live_objects": 0, "live_bytes": 0, …, "shutdown_frees": 1
    }
  }
}
```

`memory` is a snapshot of the top-level stack frame at program exit, one entry
per slot. `heap_before_shutdown` and `heap_after_shutdown` reflect heap state
immediately before and after runtime cleanup.

**Errors:** compile error, runtime error, null source or mode pointer.

---

### `tinyone_run_file_json`

```c
char *tinyone_run_file_json(
    const char *path,
    const char *mode,
    const char *inputs_json  /* nullable */
);
```

Compile the file at `path` and run it. Same response shape as
`tinyone_run_source_json`.

---

### `tinyone_run_artifact_json`

```c
char *tinyone_run_artifact_json(
    const char *artifact_json,
    const char *mode,
    const char *inputs_json  /* nullable */
);
```

Run a pre-compiled artifact without re-compiling from source. `artifact_json`
must be the raw artifact object (the `value.artifact` field from a compile
response), not the outer response envelope.

**Byte limit:** `artifact_json` must be no larger than 8 MiB (excluding the
trailing NUL). The limit is enforced by scanning the C string before parsing;
oversized inputs return a `"compile"` error without any JSON parsing.

The artifact is verified again before execution; a tampered or truncated
artifact returns a verification error.

---

### `tinyone_jit_listing_json`

```c
char *tinyone_jit_listing_json(const char *artifact_json);
```

Compile the artifact through the JIT tier and return its assembly listing as
a text string. The same 8 MiB byte limit applies.

**Success:**
```json
{
  "ok": true,
  "value": {
    "listing": "; tinyone adaptive-jit a1b2c3d4e5f60718\n; chunks=1 ops=3 …\n.chunk 0 main …\n  0000 push.i 42\n  0001 print\n  0002 halt\n"
  }
}
```

## Example: Running a Program from C

```c
#include "tinylang.h"
#include <stdio.h>
#include <string.h>

int main(void) {
    /* Compile and run inline source */
    char *result = tinyone_run_source_json(
        "let x = 6 * 7\nprint x",
        "jit",
        NULL
    );
    if (result == NULL) {
        fprintf(stderr, "tinyone returned NULL\n");
        return 1;
    }
    if (strstr(result, "\"ok\":true") == NULL) {
        fprintf(stderr, "error: %s\n", result);
        tinyone_free_string(result);
        return 1;
    }
    printf("%s\n", result);
    tinyone_free_string(result);
    return 0;
}
```

## Example: Compile Once, Run with Inputs

```c
/* Compile to artifact */
char *compiled = tinyone_compile_source_json(
    "let n = read_int()\nprint n * n"
);
/* Extract artifact field from the JSON response (use a JSON library) */
/* ... extract artifact_json string ... */

/* Run with a pre-supplied input */
char *run1 = tinyone_run_artifact_json(artifact_json, "jit", "[\"7\"]");
char *run2 = tinyone_run_artifact_json(artifact_json, "jit", "[\"12\"]");

tinyone_free_string(run1);
tinyone_free_string(run2);
tinyone_free_string(compiled);
```

## Error Handling Pattern

```c
static int check_ok(const char *label, char *response) {
    if (response == NULL) {
        fprintf(stderr, "%s: NULL response\n", label);
        return 0;
    }
    if (strstr(response, "\"ok\":true") != NULL) return 1;
    fprintf(stderr, "%s failed: %s\n", label, response);
    return 0;
}

char *r = tinyone_run_source_json("print 1", "vm", NULL);
if (!check_ok("run", r)) { tinyone_free_string(r); return 1; }
tinyone_free_string(r);
```

## Threading

TinyOne has no global mutable state in its public API. Each call is fully
self-contained. You may call FFI functions from multiple threads simultaneously
**if each call uses a distinct set of arguments** — there is no shared mutable
state between calls. Do not share a `char *` response pointer across threads
without synchronization; free it from the same thread that called the function,
or transfer ownership with appropriate synchronization.

`JitCache` maintains mutable state internally and is not thread-safe. Do not
share a `JitCache` instance across threads without external synchronization.
The FFI `run_*` functions each create a fresh `JitCache` per call, so they are
independently safe.

## Known Limitations

- `tinyone_free_string` calls `CString::from_raw` without a `catch_unwind`
  guard. Passing a double-freed pointer or a non-NUL-terminated pointer is
  undefined behavior rather than a clean panic. Obey the ownership contract.
- Future void-returning entry points cannot use the internal `respond()` helper.
  Any new `void extern "C"` function added to the library must install its own
  `catch_unwind` guard.
- The ABI is UNSTABLE. JSON response schemas, function signatures, and the
  header layout may change before v1.
